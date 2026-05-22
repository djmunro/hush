//! Menubar status item + settings window. Built on objc2/AppKit.

#![allow(deprecated)]

use std::cell::{Cell, OnceCell, RefCell};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBezelStyle, NSBox,
    NSBoxType, NSButton, NSColor, NSControlSize, NSControlStateValueOff, NSControlStateValueOn,
    NSFont, NSLayoutAttribute, NSLineBreakMode, NSMenu, NSMenuItem, NSPopUpButton, NSScrollView,
    NSStackView,
    NSStackViewDistribution, NSStatusBar, NSStatusItem, NSTextField,
    NSTextView,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    ns_string, MainThreadMarker, NSEdgeInsets, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};

use crate::autostart;
use crate::config::{self, ParakeetModel, Shortcut};
use crate::dictation::Trigger;
use crate::icon;
use crate::overlay::OverlayState;
use crate::perms::{self, MicState, PermStatus};
use crate::shortcut::ShortcutMonitor;

const VARIABLE_STATUS_ITEM_LENGTH: CGFloat = -1.0;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_HASH: &str = env!("HUSH_GIT_HASH");
const PARSER_DEFAULT_SCRIPT: &str = r#"const s = input.trim().replace(/[.?]+$/, "");
if (!s) return s;
if (s.startsWith("I ")) return s;
return `${s[0].toLowerCase()}${s.slice(1)}`;"#;

#[derive(Default)]
pub struct ControllerIvars {
    status_item: OnceCell<Retained<NSStatusItem>>,
    settings_window: OnceCell<Retained<NSWindow>>,
    mic_status_label: OnceCell<Retained<NSTextField>>,
    mic_button: OnceCell<Retained<NSButton>>,
    accessibility_status_label: OnceCell<Retained<NSTextField>>,
    accessibility_button: OnceCell<Retained<NSButton>>,
    accessibility_waiting: Cell<bool>,
    accessibility_wait_timer: RefCell<Option<Retained<NSTimer>>>,
    autostart_checkbox: OnceCell<Retained<NSButton>>,
    parakeet_model_popup: OnceCell<Retained<NSPopUpButton>>,
    parakeet_model_status_label: OnceCell<Retained<NSTextField>>,
    model_refresh_timer: RefCell<Option<Retained<NSTimer>>>,
    parser_enabled_checkbox: OnceCell<Retained<NSButton>>,
    parser_editor: OnceCell<Retained<NSTextView>>,
    parser_apply_button: OnceCell<Retained<NSButton>>,
    parser_reset_button: OnceCell<Retained<NSButton>>,
    parser_script_snapshot: RefCell<String>,
    parser_enabled_snapshot: Cell<bool>,
    trigger_hub: OnceCell<Arc<Mutex<Sender<Trigger>>>>,
    overlay_state: OnceCell<Arc<Mutex<OverlayState>>>,
    shortcut_label: OnceCell<Retained<NSTextField>>,
    shortcut_button: OnceCell<Retained<NSButton>>,
    shortcut_recording: Cell<bool>,
    shortcut: RefCell<Option<Shortcut>>,
    monitor: RefCell<Option<ShortcutMonitor>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HushAppController"]
    #[ivars = ControllerIvars]
    pub struct AppController;

    impl AppController {
        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            self.show_settings_window();
        }

        #[unsafe(method(grantMicrophone:))]
        fn grant_microphone(&self, _sender: Option<&AnyObject>) {
            match PermStatus::check().microphone {
                MicState::Granted => {}
                MicState::NotDetermined => {
                    // First-time prompt — fires the standard system mic dialog.
                    perms::request_microphone(|_granted| {
                        // Delivered on a bg queue. We refresh labels when
                        // the window regains focus via the observer.
                    });
                }
                MicState::Denied => {
                    // Already denied once — system dialog won't reshow.
                    perms::open_microphone_pane();
                }
            }
            self.refresh_perm_labels();
        }

        #[unsafe(method(grantAccessibility:))]
        fn grant_accessibility(&self, _sender: Option<&AnyObject>) {
            perms::request_accessibility();
            self.start_accessibility_wait();
        }

        #[unsafe(method(accessibilityWaitTimeout:))]
        fn accessibility_wait_timeout(&self, _timer: Option<&AnyObject>) {
            self.ivars().accessibility_waiting.set(false);
            self.ivars().accessibility_wait_timer.replace(None);
            self.refresh_perm_labels();
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            // We deliberately skip NSApplication::terminate / libc::exit:
            // both invoke C++ atexit destructors, and ggml-metal's
            // teardown asserts that its residency set is empty — which
            // we can't guarantee while the worker thread may still hold
            // Any in-flight transcriber resources are torn down with the process.
            // Going straight to _exit avoids atexit teardown races.
            extern "C" {
                fn _exit(code: i32) -> !;
            }
            unsafe { _exit(0) };
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _note: Option<&NSNotification>) {
            self.refresh_perm_labels();
        }

        #[unsafe(method(appDidBecomeActive:))]
        fn app_did_become_active(&self, _note: Option<&NSNotification>) {
            self.refresh_perm_labels();
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: Option<&AnyObject>) {
            self.refresh_perm_labels();
        }

        #[unsafe(method(recordShortcut:))]
        fn record_shortcut(&self, _sender: Option<&AnyObject>) {
            self.start_shortcut_recording();
        }

        #[unsafe(method(changeParakeetModel:))]
        fn change_parakeet_model(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|s| s.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let model = match popup.indexOfSelectedItem() {
                0 => ParakeetModel::V06b,
                1 => ParakeetModel::V11b,
                _ => return,
            };
            let mut cfg = config::load();
            let is_same = cfg.parakeet_model == model;
            let is_error = matches!(crate::audio::get_download_status(), crate::audio::DownloadStatus::Error(_));

            if is_same && !is_error {
                return;
            }

            cfg.parakeet_model = model;
            if let Err(e) = config::save(&cfg) {
                eprintln!("[hush] failed to save model config: {e}");
            }

            crate::audio::set_download_status(crate::audio::DownloadStatus::Idle);
            self.refresh_parakeet_model();

            if let Some(hub_arc) = self.ivars().trigger_hub.get() {
                let (new_tx, new_rx) = std::sync::mpsc::channel();
                if let Some(overlay_mutex) = self.ivars().overlay_state.get() {
                    let overlay = overlay_mutex.clone();
                    let dictation = crate::dictation::Dictation::production(&cfg, overlay);
                    dictation.start_processing(new_rx);
                    let mut guard = hub_arc.lock().unwrap();
                    *guard = new_tx;
                }
            }
        }

        #[unsafe(method(modelTimerTick:))]
        fn model_timer_tick(&self, _timer: Option<&AnyObject>) {
            if let Some(win) = self.ivars().settings_window.get() {
                if !win.isVisible() {
                    if let Some(t) = self.ivars().model_refresh_timer.replace(None) {
                        t.invalidate();
                    }
                    return;
                }
            }
            self.refresh_parakeet_model();
        }

        #[unsafe(method(toggleAutostart:))]
        fn toggle_autostart(&self, sender: Option<&AnyObject>) {
            let want_on = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|b| b.state() == NSControlStateValueOn)
                .unwrap_or(false);
            let result = if want_on {
                autostart::enable()
            } else {
                autostart::disable()
            };
            if let Err(e) = result {
                eprintln!("[hush] autostart toggle failed: {e}");
            }
            self.refresh_autostart();
        }

        #[unsafe(method(toggleCustomParser:))]
        fn toggle_custom_parser(&self, sender: Option<&AnyObject>) {
            let want_enabled = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|b| b.state() == NSControlStateValueOn)
                .unwrap_or(false);
            let mut cfg = config::load();
            cfg.custom_parser.enabled = want_enabled;
            if let Err(e) = config::save(&cfg) {
                eprintln!("[hush] failed to save parser config: {e}");
            } else {
                self.sync_parser_snapshot(&cfg.custom_parser);
            }
            self.update_parser_buttons();
        }

        #[unsafe(method(applyCustomParser:))]
        fn apply_custom_parser(&self, _sender: Option<&AnyObject>) {
            let cfg = self.current_parser_config_from_ui();
            if let Err(e) = config::save(&cfg) {
                eprintln!("[hush] failed to save parser config: {e}");
            } else {
                self.sync_parser_snapshot(&cfg.custom_parser);
            }
            self.update_parser_buttons();
        }

        #[unsafe(method(resetCustomParser:))]
        fn reset_custom_parser(&self, _sender: Option<&AnyObject>) {
            let script = self.ivars().parser_script_snapshot.borrow().clone();
            let enabled = self.ivars().parser_enabled_snapshot.get();
            if let Some(checkbox) = self.ivars().parser_enabled_checkbox.get() {
                checkbox.setState(if enabled {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
            if let Some(editor) = self.ivars().parser_editor.get() {
                let text = NSString::from_str(&script);
                editor.setString(&text);
            }
            self.update_parser_buttons();
        }

        #[unsafe(method(markCustomParserDirty:))]
        fn mark_custom_parser_dirty(&self, _notification: Option<&NSNotification>) {
            self.update_parser_buttons();
        }

        #[unsafe(method(defaultCustomParser:))]
        fn default_custom_parser(&self, _sender: Option<&AnyObject>) {
            if let Some(editor) = self.ivars().parser_editor.get() {
                let text = NSString::from_str(PARSER_DEFAULT_SCRIPT);
                editor.setString(&text);
            }
            self.update_parser_buttons();
        }
    }

    unsafe impl NSObjectProtocol for AppController {}
);

impl AppController {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let _ = mtm;
        let this = Self::alloc().set_ivars(ControllerIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    fn show_settings_window(&self) {
        if let Some(win) = self.ivars().settings_window.get() {
            unsafe {
                let mtm = MainThreadMarker::new_unchecked();
                let app = NSApplication::sharedApplication(mtm);
                app.activateIgnoringOtherApps(true);
                win.makeKeyAndOrderFront(None);
            }
            self.refresh_perm_labels();
            self.refresh_parakeet_model();
        }
    }

    fn refresh_autostart(&self) {
        let enabled = autostart::is_enabled();
        if let Some(checkbox) = self.ivars().autostart_checkbox.get() {
            checkbox.setState(if enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }

    fn refresh_parakeet_model(&self) {
        let cfg = config::load();
        let Some(popup) = self.ivars().parakeet_model_popup.get() else {
            return;
        };
        let index = match cfg.parakeet_model {
            ParakeetModel::V06b => 0,
            ParakeetModel::V11b => 1,
        };
        popup.selectItemAtIndex(index);
        self.update_model_status(cfg.parakeet_model);
    }

    fn start_model_refresh_timer(&self) {
        if self.ivars().model_refresh_timer.borrow().is_some() {
            return;
        }
        let timer = unsafe {
            let target: &AnyObject = self;
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0,
                target,
                sel!(modelTimerTick:),
                None,
                true,
            )
        };
        self.ivars().model_refresh_timer.replace(Some(timer));
    }

    fn stop_model_refresh_timer(&self) {
        if let Some(t) = self.ivars().model_refresh_timer.replace(None) {
            t.invalidate();
        }
    }

    fn update_model_status(&self, model: ParakeetModel) {
        let is_cached = crate::audio::is_model_cached(model);
        if let Some(label) = self.ivars().parakeet_model_status_label.get() {
            if is_cached {
                label.setStringValue(ns_string!("✓ Downloaded"));
                label.setTextColor(Some(&NSColor::systemGreenColor()));
                self.stop_model_refresh_timer();
            } else {
                match crate::audio::get_download_status() {
                    crate::audio::DownloadStatus::QueryingModelInfo => {
                        label.setStringValue(ns_string!("Querying model info…"));
                        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                        self.start_model_refresh_timer();
                    }
                    crate::audio::DownloadStatus::Downloading { file, downloaded_bytes, total_bytes } => {
                        let current_str = format_bytes(downloaded_bytes);
                        let total_str = format_bytes(total_bytes);
                        let status_str = format!("Downloading {file} ({current_str}/{total_str})…");
                        label.setStringValue(&NSString::from_str(&status_str));
                        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                        self.start_model_refresh_timer();
                    }
                    crate::audio::DownloadStatus::Error(err) => {
                        let status_str = format!("Error: {err}");
                        label.setStringValue(&NSString::from_str(&status_str));
                        label.setTextColor(Some(&NSColor::systemRedColor()));
                        self.stop_model_refresh_timer();
                    }
                    crate::audio::DownloadStatus::Idle => {
                        label.setStringValue(ns_string!("Pending download"));
                        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                        self.start_model_refresh_timer();
                    }
                }
            }
        }
    }

    fn refresh_parser(&self) {
        let cfg = config::load();
        if let Some(checkbox) = self.ivars().parser_enabled_checkbox.get() {
            checkbox.setState(if cfg.custom_parser.enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        if let Some(editor) = self.ivars().parser_editor.get() {
            let text = NSString::from_str(&cfg.custom_parser.script);
            editor.setString(&text);
        }
        self.sync_parser_snapshot(&cfg.custom_parser);
        self.update_parser_buttons();
    }

    fn sync_parser_snapshot(&self, cfg: &crate::config::CustomParserConfig) {
        self.ivars().parser_script_snapshot.replace(cfg.script.clone());
        self.ivars().parser_enabled_snapshot.set(cfg.enabled);
    }

    fn current_parser_config_from_ui(&self) -> crate::config::Config {
        let mut cfg = config::load();
        if let Some(checkbox) = self.ivars().parser_enabled_checkbox.get() {
            cfg.custom_parser.enabled = checkbox.state() == NSControlStateValueOn;
        }
        if let Some(editor) = self.ivars().parser_editor.get() {
            cfg.custom_parser.script = editor.string().to_string();
        }
        cfg
    }

    fn update_parser_buttons(&self) {
        let Some(editor) = self.ivars().parser_editor.get() else { return };
        let current_script = editor.string().to_string();
        let current_enabled = self
            .ivars()
            .parser_enabled_checkbox
            .get()
            .is_some_and(|checkbox| checkbox.state() == NSControlStateValueOn);
        let saved_script = self.ivars().parser_script_snapshot.borrow().as_str().to_string();
        let saved_enabled = self.ivars().parser_enabled_snapshot.get();

        let dirty = current_script != saved_script || current_enabled != saved_enabled;

        if let Some(apply) = self.ivars().parser_apply_button.get() {
            apply.setEnabled(dirty);
        }
        if let Some(reset) = self.ivars().parser_reset_button.get() {
            reset.setEnabled(dirty);
        }
    }

    fn refresh_perm_labels(&self) {
        let status = PermStatus::check();
        let mic_granted = status.mic_granted();

        // If Accessibility flipped to granted during the wait
        // window, clear the wait state so the UI updates immediately.
        if status.accessibility && self.ivars().accessibility_waiting.get() {
            self.ivars().accessibility_waiting.set(false);
            if let Some(t) = self.ivars().accessibility_wait_timer.replace(None) {
                t.invalidate();
            }
        }

        unsafe {
            if let Some(label) = self.ivars().mic_status_label.get() {
                label.setStringValue(&mic_status_text(status.microphone));
                label.setTextColor(Some(&perm_color(mic_granted)));
            }
            if let Some(button) = self.ivars().mic_button.get() {
                let title = match status.microphone {
                    MicState::Granted => ns_string!("Granted"),
                    MicState::NotDetermined => ns_string!("Allow microphone"),
                    MicState::Denied => ns_string!("Open System Settings…"),
                };
                button.setTitle(title);
                button.setEnabled(!mic_granted);
            }
            if let Some(label) = self.ivars().accessibility_status_label.get() {
                label.setStringValue(&perm_status_text(status.accessibility));
                label.setTextColor(Some(&perm_color(status.accessibility)));
            }
            if let Some(button) = self.ivars().accessibility_button.get() {
                self.apply_grant_button(
                    button,
                    status.accessibility,
                    self.ivars().accessibility_waiting.get(),
                    ns_string!("Open Accessibility…"),
                );
            }
        }
    }

    unsafe fn apply_grant_button(
        &self,
        button: &NSButton,
        granted: bool,
        waiting: bool,
        idle_title: &NSString,
    ) {
        let title = if granted {
            ns_string!("Granted")
        } else if waiting {
            ns_string!("Waiting…")
        } else {
            idle_title
        };
        button.setTitle(title);
        button.setEnabled(!granted && !waiting);
    }

    fn refresh_shortcut_label(&self) {
        let cur = self
            .ivars()
            .shortcut
            .borrow()
            .as_ref()
            .map(|s| s.pretty())
            .unwrap_or_else(|| "(none)".to_string());
        if let Some(label) = self.ivars().shortcut_label.get() {
            let s = NSString::from_str(&cur);
            label.setStringValue(&s);
        }
        if let Some(button) = self.ivars().shortcut_button.get() {
            let recording = self.ivars().shortcut_recording.get();
            button.setTitle(if recording {
                ns_string!("Press shortcut…")
            } else {
                ns_string!("Record…")
            });
            button.setEnabled(!recording);
        }
    }

    fn start_shortcut_recording(&self) {
        self.ivars().shortcut_recording.set(true);
        self.refresh_shortcut_label();

        // Hand a raw pointer to the callback. The AppController is owned
        // by the menubar / settings window for the lifetime of the
        // process, so this address stays valid.
        let weak: usize = (self as *const AppController) as usize;

        if let Some(monitor) = self.ivars().monitor.borrow().as_ref() {
            monitor.start_recording(move |captured| {
                let ptr = weak as *const AppController;
                if ptr.is_null() {
                    return;
                }
                unsafe {
                    let controller: &AppController = &*ptr;
                    controller.finish_shortcut_recording(captured);
                }
            });
        } else {
            self.ivars().shortcut_recording.set(false);
            self.refresh_shortcut_label();
        }
    }

    fn finish_shortcut_recording(&self, captured: Option<Shortcut>) {
        self.ivars().shortcut_recording.set(false);

        if let Some(sc) = captured {
            *self.ivars().shortcut.borrow_mut() = Some(sc.clone());
            if let Some(monitor) = self.ivars().monitor.borrow().as_ref() {
                monitor.set_binding(sc.clone());
            }
            let mut cfg = config::load();
            cfg.shortcut = sc;
            if let Err(e) = config::save(&cfg) {
                eprintln!("[hush] save config: {e}");
            }
        }
        // Cancelled (Esc): leave existing binding untouched, just refresh UI.
        self.refresh_shortcut_label();
    }

    pub fn set_initial_shortcut(&self, sc: Shortcut, monitor: Option<ShortcutMonitor>) {
        *self.ivars().shortcut.borrow_mut() = Some(sc);
        *self.ivars().monitor.borrow_mut() = monitor;
    }

    fn start_accessibility_wait(&self) {
        self.ivars().accessibility_waiting.set(true);
        let timer = unsafe {
            let target: &AnyObject = self;
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                10.0,
                target,
                sel!(accessibilityWaitTimeout:),
                None,
                false,
            )
        };
        if let Some(prev) = self.ivars().accessibility_wait_timer.replace(Some(timer)) {
            prev.invalidate();
        }
        self.refresh_perm_labels();
    }
}

unsafe fn mic_status_text(state: MicState) -> Retained<NSString> {
    let text = match state {
        MicState::Granted => "✓  Granted",
        MicState::NotDetermined => "•  Not yet asked",
        MicState::Denied => "✗  Denied",
    };
    NSString::from_str(text)
}

unsafe fn perm_status_text(granted: bool) -> Retained<NSString> {
    let text = if granted { "✓  Granted" } else { "✗  Not granted" };
    NSString::from_str(text)
}

unsafe fn perm_color(granted: bool) -> Retained<NSColor> {
    if granted {
        NSColor::systemGreenColor()
    } else {
        NSColor::secondaryLabelColor()
    }
}

pub struct UiHandles {
    pub controller: Retained<AppController>,
}

pub fn install_menubar_and_window(
    mtm: MainThreadMarker,
    initial_shortcut: Shortcut,
    monitor: Option<ShortcutMonitor>,
    trigger_hub: Arc<Mutex<Sender<Trigger>>>,
    overlay_state: Arc<Mutex<OverlayState>>,
) -> UiHandles {
    let controller = AppController::new(mtm);
    controller.set_initial_shortcut(initial_shortcut, monitor);

    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        install_main_menu(mtm, &controller);

        // Status item
        let status_bar = NSStatusBar::systemStatusBar();
        let status_item = status_bar.statusItemWithLength(VARIABLE_STATUS_ITEM_LENGTH);

        let icon_image = icon::build_template_icon();
        if let Some(button) = status_item.button(mtm) {
            button.setImage(Some(&icon_image));
            button.setToolTip(Some(ns_string!("hush — push-to-talk dictation")));
        }

        let menu = NSMenu::new(mtm);
        let version_item = NSMenuItem::new(mtm);
        version_item.setTitle(&NSString::from_str(&format!(
            "Hush {VERSION} ({GIT_HASH})"
        )));
        version_item.setEnabled(false);
        menu.addItem(&version_item);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(
            mtm,
            ns_string!("Settings…"),
            sel!(openSettings:),
            ns_string!(","),
            &controller,
        ));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(
            mtm,
            ns_string!("Quit hush"),
            sel!(quit:),
            ns_string!("q"),
            &controller,
        ));
        status_item.setMenu(Some(&menu));

        // Settings window
        let window = build_settings_window(mtm, &controller);

        let center = NSNotificationCenter::defaultCenter();
        let observer: &AnyObject = &controller;
        center.addObserver_selector_name_object(
            observer,
            sel!(windowDidBecomeKey:),
            Some(ns_string!("NSWindowDidBecomeKeyNotification")),
            Some(&*window),
        );
        // Fires whenever hush comes to the foreground (cmd-tab, click on
        // the menubar icon, etc.) — catches the "user came back from
        // System Settings" case the window-key notification misses.
        center.addObserver_selector_name_object(
            observer,
            sel!(appDidBecomeActive:),
            Some(ns_string!("NSApplicationDidBecomeActiveNotification")),
            None,
        );

        // Belt-and-suspenders: poll perm state every 1.5s so the
        // settings UI updates even while the app is in the background.
        // TCC has no notification API, so polling is the standard.
        let _timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            1.5,
            observer,
            sel!(tick:),
            None,
            true,
        );

        let _ = controller.ivars().status_item.set(status_item);
        let _ = controller.ivars().settings_window.set(window);
    }

    let _ = controller.ivars().trigger_hub.set(trigger_hub);
    let _ = controller.ivars().overlay_state.set(overlay_state);
    controller.refresh_perm_labels();
    controller.refresh_autostart();
    controller.refresh_parakeet_model();
    controller.refresh_parser();
    controller.refresh_shortcut_label();
    UiHandles { controller }
}

unsafe fn menu_item(
    mtm: MainThreadMarker,
    title: &NSString,
    action: Sel,
    key: &NSString,
    target: &AppController,
) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(title);
    item.setAction(Some(action));
    item.setKeyEquivalent(key);
    let target_obj: &AnyObject = target;
    item.setTarget(Some(target_obj));
    item
}

unsafe fn install_main_menu(mtm: MainThreadMarker, controller: &AppController) {
    let app = NSApplication::sharedApplication(mtm);
    let controller_obj: &AnyObject = controller;
    let main_menu = NSMenu::new(mtm);

    let app_menu_item = NSMenuItem::new(mtm);
    app_menu_item.setTitle(ns_string!("hush"));
    let app_menu = NSMenu::new(mtm);

    let about_item = NSMenuItem::new(mtm);
    about_item.setTitle(ns_string!("About hush"));
    about_item.setEnabled(false);

    let settings_item = NSMenuItem::new(mtm);
    settings_item.setTitle(ns_string!("Settings…"));
    settings_item.setAction(Some(sel!(openSettings:)));
    settings_item.setTarget(Some(controller_obj));
    settings_item.setKeyEquivalent(ns_string!(","));

    let quit_item = NSMenuItem::new(mtm);
    quit_item.setTitle(ns_string!("Quit hush"));
    quit_item.setAction(Some(sel!(quit:)));
    quit_item.setTarget(Some(controller_obj));
    quit_item.setKeyEquivalent(ns_string!("q"));

    app_menu.addItem(&about_item);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&settings_item);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&quit_item);
    app_menu_item.setSubmenu(Some(&app_menu));

    let edit_menu_item = NSMenuItem::new(mtm);
    edit_menu_item.setTitle(ns_string!("Edit"));
    let edit_menu = NSMenu::new(mtm);

    let undo_item = NSMenuItem::new(mtm);
    undo_item.setTitle(ns_string!("Undo"));
    undo_item.setAction(Some(sel!(undo:)));
    undo_item.setTarget(None);
    undo_item.setKeyEquivalent(ns_string!("z"));
    edit_menu.addItem(&undo_item);

    let redo_item = NSMenuItem::new(mtm);
    redo_item.setTitle(ns_string!("Redo"));
    redo_item.setAction(Some(sel!(redo:)));
    redo_item.setTarget(None);
    redo_item.setKeyEquivalent(ns_string!("Z"));
    edit_menu.addItem(&redo_item);

    let cut_item = NSMenuItem::new(mtm);
    cut_item.setTitle(ns_string!("Cut"));
    cut_item.setAction(Some(sel!(cut:)));
    cut_item.setTarget(None);
    cut_item.setKeyEquivalent(ns_string!("x"));
    edit_menu.addItem(&cut_item);

    let copy_item = NSMenuItem::new(mtm);
    copy_item.setTitle(ns_string!("Copy"));
    copy_item.setAction(Some(sel!(copy:)));
    copy_item.setTarget(None);
    copy_item.setKeyEquivalent(ns_string!("c"));
    edit_menu.addItem(&copy_item);

    let paste_item = NSMenuItem::new(mtm);
    paste_item.setTitle(ns_string!("Paste"));
    paste_item.setAction(Some(sel!(paste:)));
    paste_item.setTarget(None);
    paste_item.setKeyEquivalent(ns_string!("v"));
    edit_menu.addItem(&paste_item);

    let select_all_item = NSMenuItem::new(mtm);
    select_all_item.setTitle(ns_string!("Select All"));
    select_all_item.setAction(Some(sel!(selectAll:)));
    select_all_item.setTarget(None);
    select_all_item.setKeyEquivalent(ns_string!("a"));
    edit_menu.addItem(&select_all_item);

    edit_menu_item.setSubmenu(Some(&edit_menu));

    main_menu.addItem(&app_menu_item);
    main_menu.addItem(&edit_menu_item);

    app.setMainMenu(Some(&main_menu));
}

unsafe fn build_settings_window(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(520.0, 560.0));

    let window: Retained<NSWindow> = NSWindow::initWithContentRect_styleMask_backing_defer(
        NSWindow::alloc(mtm),
        frame,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    window.setTitle(ns_string!("hush"));
    window.setReleasedWhenClosed(false);
    window.center();

    let content_view = window.contentView().expect("content view");

    let stack = NSStackView::new(mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setSpacing(14.0);
    stack.setAlignment(NSLayoutAttribute::Leading);
    stack.setEdgeInsets(NSEdgeInsets {
        top: 22.0,
        left: 24.0,
        bottom: 22.0,
        right: 24.0,
    });
    stack.setDistribution(NSStackViewDistribution::Fill);
    stack.setTranslatesAutoresizingMaskIntoConstraints(false);

    let title = make_label(mtm, ns_string!("hush"), 22.0, true);
    stack.addArrangedSubview(&title);

    let subtitle = make_label(
        mtm,
        ns_string!("Hold your shortcut, speak, release to paste."),
        12.0,
        false,
    );
    subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
    stack.addArrangedSubview(&subtitle);

    let general_heading = make_label(mtm, ns_string!("General"), 14.0, true);
    stack.addArrangedSubview(&general_heading);

    let autostart_box = build_autostart_card(mtm, controller);
    let model_box = build_parakeet_model_card(mtm, controller);
    add_card_row(mtm, &stack, &autostart_box, &model_box);

    let shortcut_card = build_shortcut_card(mtm, controller);
    add_card(&stack, &shortcut_card);

    let perms_heading = make_label(mtm, ns_string!("Permissions"), 14.0, true);
    stack.addArrangedSubview(&perms_heading);

    let mic_card = build_card(
        mtm,
        ns_string!("Microphone"),
        ns_string!("Captured via the standard system mic prompt — no settings detour required."),
        ns_string!("Allow microphone"),
        sel!(grantMicrophone:),
        controller,
        |labels| {
            let _ = controller.ivars().mic_status_label.set(labels.status.clone());
            let _ = controller.ivars().mic_button.set(labels.button.clone());
        },
    );
    let acc_card = build_card(
        mtm,
        ns_string!("Accessibility"),
        ns_string!("Detects your shortcut globally and pastes transcripts via Cmd+V."),
        ns_string!("Open Accessibility…"),
        sel!(grantAccessibility:),
        controller,
        |labels| {
            let _ = controller.ivars().accessibility_status_label.set(labels.status.clone());
            let _ = controller.ivars().accessibility_button.set(labels.button.clone());
        },
    );
    add_card_row(mtm, &stack, &mic_card, &acc_card);

    let parser_heading = make_label(mtm, ns_string!("Custom Parser"), 14.0, true);
    stack.addArrangedSubview(&parser_heading);

    let parser_box = build_parser_card(mtm, controller);
    add_card(&stack, &parser_box);

    let footer = make_label(
        mtm,
        ns_string!("hush keeps running in the menubar even without permissions — grant them when you're ready."),
        11.0,
        false,
    );
    footer.setTextColor(Some(&NSColor::tertiaryLabelColor()));
    footer.setUsesSingleLineMode(false);
    footer.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    stack.addArrangedSubview(&footer);

    content_view.addSubview(&stack);

    let constraints = [
        stack
            .leadingAnchor()
            .constraintEqualToAnchor(&content_view.leadingAnchor()),
        stack
            .trailingAnchor()
            .constraintEqualToAnchor(&content_view.trailingAnchor()),
        stack
            .topAnchor()
            .constraintEqualToAnchor(&content_view.topAnchor()),
        stack
            .bottomAnchor()
            .constraintEqualToAnchor(&content_view.bottomAnchor()),
    ];
    for c in &constraints {
        c.setActive(true);
    }

    window
}

struct CardLabels {
    status: Retained<NSTextField>,
    button: Retained<NSButton>,
}

unsafe fn build_card(
    mtm: MainThreadMarker,
    title: &NSString,
    description: &NSString,
    button_title: &NSString,
    action: Sel,
    target: &AppController,
    register: impl FnOnce(&CardLabels),
) -> Retained<NSBox> {
    let box_view = NSBox::new(mtm);
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setBorderType(objc2_app_kit::NSBorderType::LineBorder);
    box_view.setBorderColor(&NSColor::separatorColor());
    box_view.setCornerRadius(10.0);
    box_view.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    box_view.setContentViewMargins(NSSize::new(0.0, 0.0));
    box_view.setTranslatesAutoresizingMaskIntoConstraints(false);

    let inner = NSStackView::new(mtm);
    inner.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    inner.setSpacing(8.0);
    inner.setAlignment(NSLayoutAttribute::Leading);
    inner.setEdgeInsets(NSEdgeInsets {
        top: 14.0,
        left: 16.0,
        bottom: 14.0,
        right: 16.0,
    });
    inner.setDistribution(NSStackViewDistribution::Fill);
    inner.setTranslatesAutoresizingMaskIntoConstraints(false);

    // Header row: title on left, status pill on right.
    let header = NSStackView::new(mtm);
    header.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    header.setSpacing(8.0);
    header.setDistribution(NSStackViewDistribution::Fill);
    header.setAlignment(NSLayoutAttribute::CenterY);
    header.setTranslatesAutoresizingMaskIntoConstraints(false);

    let title_label = make_label(mtm, title, 14.0, true);
    let spacer = NSView::new(mtm);
    spacer.setTranslatesAutoresizingMaskIntoConstraints(false);
    let status_label = make_label(mtm, ns_string!("…"), 12.0, false);
    status_label.setTextColor(Some(&NSColor::secondaryLabelColor()));

    header.addArrangedSubview(&title_label);
    header.addArrangedSubview(&spacer);
    header.addArrangedSubview(&status_label);

    inner.addArrangedSubview(&header);

    let desc = make_label(mtm, description, 11.0, false);
    desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    desc.setUsesSingleLineMode(false);
    desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&desc);

    let button = NSButton::new(mtm);
    button.setTitle(button_title);
    button.setBezelStyle(NSBezelStyle::Rounded);
    button.setControlSize(NSControlSize::Regular);
    let target_obj: &AnyObject = target;
    button.setTarget(Some(target_obj));
    button.setAction(Some(action));
    inner.addArrangedSubview(&button);

    box_view.setContentView(Some(&inner));

    // NSBox doesn't auto-pin its contentView via autolayout. Without
    // these, the inner stack collapses and the box has zero height.
    let inner_view: &NSView = &inner;
    let box_super: &NSView = &box_view;
    pin_view_to_parent(inner_view, box_super);

    // Make header expand to fill row width so the status label hits the right edge.
    let header_view: &NSView = &header;
    let inner_super: &NSView = &inner;
    let header_width = header_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_super.widthAnchor(), -32.0);
    header_width.setActive(true);

    // Description spans full inner width too (for wrapping).
    let desc_view: &NSView = &desc;
    let desc_width = desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_super.widthAnchor(), -32.0);
    desc_width.setActive(true);

    let labels = CardLabels {
        status: status_label,
        button,
    };
    register(&labels);

    box_view
}

unsafe fn build_shortcut_card(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSBox> {
    let box_view = NSBox::new(mtm);
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setBorderType(objc2_app_kit::NSBorderType::LineBorder);
    box_view.setBorderColor(&NSColor::separatorColor());
    box_view.setCornerRadius(10.0);
    box_view.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    box_view.setContentViewMargins(NSSize::new(0.0, 0.0));
    box_view.setTranslatesAutoresizingMaskIntoConstraints(false);

    let inner = NSStackView::new(mtm);
    inner.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    inner.setSpacing(8.0);
    inner.setAlignment(NSLayoutAttribute::Leading);
    inner.setEdgeInsets(NSEdgeInsets {
        top: 14.0,
        left: 16.0,
        bottom: 14.0,
        right: 16.0,
    });
    inner.setDistribution(NSStackViewDistribution::Fill);
    inner.setTranslatesAutoresizingMaskIntoConstraints(false);

    let header = NSStackView::new(mtm);
    header.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    header.setSpacing(8.0);
    header.setDistribution(NSStackViewDistribution::Fill);
    header.setAlignment(NSLayoutAttribute::CenterY);
    header.setTranslatesAutoresizingMaskIntoConstraints(false);

    let title_label = make_label(mtm, ns_string!("Push-to-talk"), 14.0, true);
    let spacer = NSView::new(mtm);
    spacer.setTranslatesAutoresizingMaskIntoConstraints(false);
    let value_label = make_label(mtm, ns_string!("…"), 13.0, true);

    header.addArrangedSubview(&title_label);
    header.addArrangedSubview(&spacer);
    header.addArrangedSubview(&value_label);

    inner.addArrangedSubview(&header);

    let desc = make_label(
        mtm,
        ns_string!("Hold this combo to dictate. Click \"Record…\" then press the keys you want — modifiers (incl. left/right side) and one optional non-modifier key. Press Esc to cancel."),
        11.0,
        false,
    );
    desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    desc.setUsesSingleLineMode(false);
    desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&desc);

    let button = NSButton::new(mtm);
    button.setTitle(ns_string!("Record…"));
    button.setBezelStyle(NSBezelStyle::Rounded);
    button.setControlSize(NSControlSize::Regular);
    let target_obj: &AnyObject = controller;
    button.setTarget(Some(target_obj));
    button.setAction(Some(sel!(recordShortcut:)));
    inner.addArrangedSubview(&button);

    box_view.setContentView(Some(&inner));

    let inner_view: &NSView = &inner;
    let box_super: &NSView = &box_view;
    pin_view_to_parent(inner_view, box_super);

    let header_view: &NSView = &header;
    header_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let desc_view: &NSView = &desc;
    desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let _ = controller.ivars().shortcut_label.set(value_label);
    let _ = controller.ivars().shortcut_button.set(button);

    box_view
}

unsafe fn build_autostart_card(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSBox> {
    let box_view = NSBox::new(mtm);
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setBorderType(objc2_app_kit::NSBorderType::LineBorder);
    box_view.setBorderColor(&NSColor::separatorColor());
    box_view.setCornerRadius(10.0);
    box_view.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    box_view.setContentViewMargins(NSSize::new(0.0, 0.0));
    box_view.setTranslatesAutoresizingMaskIntoConstraints(false);

    let inner = NSStackView::new(mtm);
    inner.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    inner.setSpacing(6.0);
    inner.setAlignment(NSLayoutAttribute::Leading);
    inner.setEdgeInsets(NSEdgeInsets {
        top: 14.0,
        left: 16.0,
        bottom: 14.0,
        right: 16.0,
    });
    inner.setDistribution(NSStackViewDistribution::Fill);
    inner.setTranslatesAutoresizingMaskIntoConstraints(false);

    let checkbox = NSButton::new(mtm);
    checkbox.setButtonType(objc2_app_kit::NSButtonType::Switch);
    checkbox.setTitle(ns_string!("Open Hush at login"));
    let target_obj: &AnyObject = controller;
    checkbox.setTarget(Some(target_obj));
    checkbox.setAction(Some(sel!(toggleAutostart:)));
    inner.addArrangedSubview(&checkbox);

    let desc = make_label(
        mtm,
        ns_string!("Manages a per-user LaunchAgent that opens Hush.app at login."),
        11.0,
        false,
    );
    desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    desc.setUsesSingleLineMode(false);
    desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&desc);

    box_view.setContentView(Some(&inner));

    let inner_view: &NSView = &inner;
    let box_super: &NSView = &box_view;
    pin_view_to_parent(inner_view, box_super);

    let desc_view: &NSView = &desc;
    desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let _ = controller.ivars().autostart_checkbox.set(checkbox);

    box_view
}

unsafe fn build_parakeet_model_card(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSBox> {
    let box_view = NSBox::new(mtm);
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setBorderType(objc2_app_kit::NSBorderType::LineBorder);
    box_view.setBorderColor(&NSColor::separatorColor());
    box_view.setCornerRadius(10.0);
    box_view.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    box_view.setContentViewMargins(NSSize::new(0.0, 0.0));
    box_view.setTranslatesAutoresizingMaskIntoConstraints(false);

    let inner = NSStackView::new(mtm);
    inner.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    inner.setSpacing(6.0);
    inner.setAlignment(NSLayoutAttribute::Leading);
    inner.setEdgeInsets(NSEdgeInsets {
        top: 14.0,
        left: 16.0,
        bottom: 14.0,
        right: 16.0,
    });
    inner.setDistribution(NSStackViewDistribution::Fill);
    inner.setTranslatesAutoresizingMaskIntoConstraints(false);

    let label = make_label(mtm, ns_string!("Parakeet model"), 13.0, true);
    inner.addArrangedSubview(&label);

    let row = NSStackView::new(mtm);
    row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    row.setSpacing(10.0);
    row.setAlignment(NSLayoutAttribute::CenterY);
    row.setDistribution(NSStackViewDistribution::Fill);
    row.setTranslatesAutoresizingMaskIntoConstraints(false);

    let popup = NSPopUpButton::new(mtm);
    popup.addItemWithTitle(ns_string!("0.6B"));
    popup.addItemWithTitle(ns_string!("1.1B"));
    let target_obj: &AnyObject = controller;
    popup.setTarget(Some(target_obj));
    popup.setAction(Some(sel!(changeParakeetModel:)));
    row.addArrangedSubview(&popup);

    let status_label = make_label(mtm, ns_string!(""), 12.0, false);
    row.addArrangedSubview(&status_label);

    inner.addArrangedSubview(&row);

    let desc = make_label(
        mtm,
        ns_string!(
            "0.6B is faster and smaller; 1.1B is more accurate but slower and uses more disk (~4 GB). Loads automatically when changed."
        ),
        11.0,
        false,
    );
    desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    desc.setUsesSingleLineMode(false);
    desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&desc);

    box_view.setContentView(Some(&inner));

    let inner_view: &NSView = &inner;
    let box_super: &NSView = &box_view;
    pin_view_to_parent(inner_view, box_super);

    let desc_view: &NSView = &desc;
    desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let _ = controller.ivars().parakeet_model_popup.set(popup);
    let _ = controller.ivars().parakeet_model_status_label.set(status_label);
    controller.refresh_parakeet_model();

    box_view
}

unsafe fn build_parser_card(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSBox> {
    let box_view = NSBox::new(mtm);
    box_view.setBoxType(NSBoxType::Custom);
    box_view.setBorderType(objc2_app_kit::NSBorderType::LineBorder);
    box_view.setBorderColor(&NSColor::separatorColor());
    box_view.setCornerRadius(10.0);
    box_view.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    box_view.setContentViewMargins(NSSize::new(0.0, 0.0));
    box_view.setTranslatesAutoresizingMaskIntoConstraints(false);

    let inner = NSStackView::new(mtm);
    inner.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    inner.setSpacing(6.0);
    inner.setAlignment(NSLayoutAttribute::Leading);
    inner.setEdgeInsets(NSEdgeInsets {
        top: 14.0,
        left: 16.0,
        bottom: 14.0,
        right: 16.0,
    });
    inner.setDistribution(NSStackViewDistribution::Fill);
    inner.setTranslatesAutoresizingMaskIntoConstraints(false);

    let label = make_label(mtm, ns_string!("Custom parser"), 13.0, true);
    inner.addArrangedSubview(&label);

    let desc = make_label(
        mtm,
        ns_string!("Run a final JavaScript transform before paste. Return text or number. Return null/undefined/other values to keep original text."),
        11.0,
        false,
    );
    desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    desc.setUsesSingleLineMode(false);
    desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&desc);

    let controls = NSStackView::new(mtm);
    controls.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    controls.setSpacing(12.0);
    controls.setAlignment(NSLayoutAttribute::CenterY);
    controls.setDistribution(NSStackViewDistribution::Fill);
    controls.setTranslatesAutoresizingMaskIntoConstraints(false);

    let enabled_checkbox = NSButton::new(mtm);
    enabled_checkbox.setButtonType(objc2_app_kit::NSButtonType::Switch);
    enabled_checkbox.setTitle(ns_string!("Enable custom parser"));
    let target_obj: &AnyObject = controller;
    enabled_checkbox.setTarget(Some(target_obj));
    enabled_checkbox.setAction(Some(sel!(toggleCustomParser:)));
    controls.addArrangedSubview(&enabled_checkbox);

    let apply_button = NSButton::new(mtm);
    apply_button.setTitle(ns_string!("Apply"));
    apply_button.setBezelStyle(NSBezelStyle::Rounded);
    apply_button.setControlSize(NSControlSize::Regular);
    apply_button.setTarget(Some(target_obj));
    apply_button.setAction(Some(sel!(applyCustomParser:)));
    controls.addArrangedSubview(&apply_button);

    let default_button = NSButton::new(mtm);
    default_button.setTitle(ns_string!("Default"));
    default_button.setBezelStyle(NSBezelStyle::Rounded);
    default_button.setControlSize(NSControlSize::Regular);
    default_button.setTarget(Some(target_obj));
    default_button.setAction(Some(sel!(defaultCustomParser:)));
    controls.addArrangedSubview(&default_button);

    let reset_button = NSButton::new(mtm);
    reset_button.setTitle(ns_string!("Reset"));
    reset_button.setBezelStyle(NSBezelStyle::Rounded);
    reset_button.setControlSize(NSControlSize::Regular);
    reset_button.setTarget(Some(target_obj));
    reset_button.setAction(Some(sel!(resetCustomParser:)));
    controls.addArrangedSubview(&reset_button);

    let scroll = NSScrollView::new(mtm);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(false);
    scroll.setDrawsBackground(true);

    let editor = NSTextView::new(mtm);
    editor.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    let ns_text = NSString::from_str("");
    editor.setString(&ns_text);

    scroll.setDocumentView(Some(&editor));
    scroll.setTranslatesAutoresizingMaskIntoConstraints(false);
    let editor_height = scroll
        .heightAnchor()
        .constraintEqualToConstant(132.0);
    editor_height.setActive(true);
    inner.addArrangedSubview(&scroll);

    let editor_for_notify = editor.clone();
    let _ = controller.ivars().parser_editor.set(editor);
    let _ = controller.ivars().parser_enabled_checkbox.set(enabled_checkbox);
    let _ = controller.ivars().parser_apply_button.set(apply_button);
    let _ = controller.ivars().parser_reset_button.set(reset_button);
    controller.refresh_parser();
    inner.addArrangedSubview(&controls);

    let name = ns_string!("NSTextDidChangeNotification");
    let center = NSNotificationCenter::defaultCenter();
    center.addObserver_selector_name_object(
        controller,
        sel!(markCustomParserDirty:),
        Some(name),
        Some(&*editor_for_notify),
    );

    box_view.setContentView(Some(&inner));

    let inner_view: &NSView = &inner;
    let box_super: &NSView = &box_view;
    pin_view_to_parent(inner_view, box_super);

    let desc_view: &NSView = &desc;
    desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let scroll_view: &NSView = &scroll;
    scroll_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    box_view
}


/// Adds a card to the outer settings stack and pins its width so it
/// fills the available content area (the outer stack centers /
/// intrinsic-sizes its children otherwise).
unsafe fn add_card(stack: &NSStackView, card: &NSBox) {
    stack.addArrangedSubview(card);
    pin_row_width(stack, card);
}

unsafe fn add_card_row(
    mtm: MainThreadMarker,
    stack: &NSStackView,
    left: &NSBox,
    right: &NSBox,
) {
    let row = NSStackView::new(mtm);
    row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    row.setSpacing(12.0);
    row.setAlignment(NSLayoutAttribute::Top);
    row.setDistribution(NSStackViewDistribution::FillEqually);
    row.setTranslatesAutoresizingMaskIntoConstraints(false);
    row.addArrangedSubview(left);
    row.addArrangedSubview(right);
    stack.addArrangedSubview(&row);
    pin_row_width(stack, &row);

    left.heightAnchor()
        .constraintEqualToAnchor(&right.heightAnchor())
        .setActive(true);

    left.widthAnchor()
        .constraintEqualToAnchor(&right.widthAnchor())
        .setActive(true);
}

unsafe fn pin_row_width(stack: &NSStackView, view: &NSView) {
    view.widthAnchor()
        .constraintEqualToAnchor_constant(&stack.widthAnchor(), -48.0)
        .setActive(true);
}

unsafe fn pin_view_to_parent(child: &NSView, parent: &NSView) {
    let cs = [
        child
            .leadingAnchor()
            .constraintEqualToAnchor(&parent.leadingAnchor()),
        child
            .trailingAnchor()
            .constraintEqualToAnchor(&parent.trailingAnchor()),
        child
            .topAnchor()
            .constraintEqualToAnchor(&parent.topAnchor()),
        child
            .bottomAnchor()
            .constraintEqualToAnchor(&parent.bottomAnchor()),
    ];
    for c in &cs {
        c.setActive(true);
    }
}

unsafe fn make_label(
    mtm: MainThreadMarker,
    text: &NSString,
    size: CGFloat,
    bold: bool,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(text, mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    label.setFont(Some(&font));
    label.setSelectable(false);
    label
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < units.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.2} {}", size, units[unit_idx])
}

pub fn maybe_show_settings_at_launch(controller: &AppController) {
    let status = PermStatus::check();
    if !status.all_granted() {
        controller.show_settings_window();
    }
}

pub fn run_app(mtm: MainThreadMarker) -> ! {
    let app = NSApplication::sharedApplication(mtm);
    app.run();
    std::process::exit(0);
}
