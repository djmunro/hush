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
    NSFont, NSLayoutAttribute, NSLineBreakMode, NSMenu, NSMenuItem, NSStackView,
    NSStackViewDistribution, NSStatusBar, NSStatusItem, NSTextField, NSTextView, NSPopUpButton,
    NSScrollView,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    ns_string, MainThreadMarker, NSEdgeInsets, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};

use crate::autostart;
use crate::dictation::{Dictation, Trigger};
use crate::icon;
use crate::overlay::OverlayState;
use crate::perms::{self, MicState, PermStatus};

const VARIABLE_STATUS_ITEM_LENGTH: CGFloat = -1.0;

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
    backend_checkbox: OnceCell<Retained<NSButton>>,
    post_process_checkbox: OnceCell<Retained<NSButton>>,
    post_process_model_popup: OnceCell<Retained<NSPopUpButton>>,
    post_process_prompt_text_view: OnceCell<Retained<NSTextView>>,
    post_process_save_button: OnceCell<Retained<NSButton>>,
    post_process_status_label: OnceCell<Retained<NSTextField>>,
    trigger_hub: OnceCell<Arc<Mutex<Sender<Trigger>>>>,
    overlay_state: OnceCell<Arc<Mutex<OverlayState>>>,
    backend_switch_lock: OnceCell<Arc<Mutex<()>>>,
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
            perms::open_accessibility_pane();
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
            // the WhisperContext. Going straight to _exit avoids the
            // crash on quit.
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

        #[unsafe(method(toggleBackend:))]
        fn toggle_backend(&self, sender: Option<&AnyObject>) {
            let want_parakeet = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|b| b.state() == NSControlStateValueOn)
                .unwrap_or(false);
            crate::prefs::set_backend(if want_parakeet { "parakeet" } else { "whisper" });

            let Some(hub) = self.ivars().trigger_hub.get().cloned() else { return };
            let Some(overlay) = self.ivars().overlay_state.get().cloned() else { return };
            let Some(switch_lock) = self.ivars().backend_switch_lock.get().cloned() else { return };

            std::thread::spawn(move || {
                let _guard = switch_lock.lock().unwrap();
                let backend = crate::audio::ensure_backend_model(want_parakeet);
                let (new_tx, new_rx) = std::sync::mpsc::channel();
                Dictation::production(backend, overlay).start_processing(new_rx);
                // Dropping the old sender signals the old pipeline thread to exit.
                *hub.lock().unwrap() = new_tx;
            });
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

        #[unsafe(method(togglePostProcess:))]
        fn toggle_post_process(&self, sender: Option<&AnyObject>) {
            let enabled = sender
                .and_then(|s| s.downcast_ref::<NSButton>())
                .map(|b| b.state() == NSControlStateValueOn)
                .unwrap_or(false);
            crate::prefs::set_post_process_enabled(enabled);
            self.refresh_post_process();
        }

        #[unsafe(method(selectPostProcessModel:))]
        fn select_post_process_model(&self, sender: Option<&AnyObject>) {
            let selected = sender
                .and_then(|s| s.downcast_ref::<NSPopUpButton>())
                .and_then(|popup| popup.titleOfSelectedItem())
                .map(|title| title.to_string())
                .unwrap_or_default();
            if !selected.is_empty() {
                crate::prefs::set_post_process_model(&selected);
            }
            self.refresh_post_process();
        }

        #[unsafe(method(refreshPostProcessModels:))]
        fn refresh_post_process_models_action(&self, _sender: Option<&AnyObject>) {
            self.refresh_post_process_models();
        }

        #[unsafe(method(savePostProcessPrompt:))]
        fn save_post_process_prompt_action(&self, _sender: Option<&AnyObject>) {
            let Some(text_view) = self.ivars().post_process_prompt_text_view.get() else {
                return;
            };
            let prompt = text_view.string().to_string();
            crate::prefs::set_post_process_prompt(&prompt);
            self.refresh_post_process_prompt_save_button();
        }

        #[unsafe(method(resetPostProcessPrompt:))]
        fn reset_post_process_prompt_action(&self, _sender: Option<&AnyObject>) {
            crate::prefs::reset_post_process_prompt();
            self.refresh_post_process();
        }

        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, note: Option<&NSNotification>) {
            let changed_ptr = note
                .and_then(|n| n.object())
                .map(|obj| (&*obj) as *const AnyObject);
            let prompt_ptr = self
                .ivars()
                .post_process_prompt_text_view
                .get()
                .map(|tv| (&**tv as &AnyObject) as *const AnyObject);
            let is_prompt_editor = changed_ptr.is_some() && changed_ptr == prompt_ptr;
            if is_prompt_editor {
                self.refresh_post_process_prompt_save_button();
            }
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
                app.activate();
                win.makeKeyAndOrderFront(None);
            }
            self.refresh_perm_labels();
        }
    }

    fn refresh_backend(&self) {
        let using_parakeet = crate::prefs::get_backend() == "parakeet";
        if let Some(checkbox) = self.ivars().backend_checkbox.get() {
            checkbox.setState(if using_parakeet {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
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

    fn set_post_process_status(&self, message: &str, error: bool) {
        if let Some(label) = self.ivars().post_process_status_label.get() {
            let text = NSString::from_str(message);
            label.setStringValue(&text);
            let color = if error {
                NSColor::systemRedColor()
            } else {
                NSColor::secondaryLabelColor()
            };
            label.setTextColor(Some(&color));
        }
    }

    fn refresh_post_process(&self) {
        let enabled = crate::prefs::get_post_process_enabled();
        if let Some(checkbox) = self.ivars().post_process_checkbox.get() {
            checkbox.setState(if enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        if let Some(popup) = self.ivars().post_process_model_popup.get() {
            popup.setEnabled(enabled);
        }
        if let Some(text_view) = self.ivars().post_process_prompt_text_view.get() {
            text_view.setEditable(enabled);
            let prompt = NSString::from_str(&crate::prefs::get_post_process_prompt());
            text_view.setString(&prompt);
        }
        self.refresh_post_process_prompt_save_button();
    }

    fn refresh_post_process_prompt_save_button(&self) {
        let enabled = crate::prefs::get_post_process_enabled();
        let dirty = self
            .ivars()
            .post_process_prompt_text_view
            .get()
            .map(|text_view| text_view.string().to_string() != crate::prefs::get_post_process_prompt())
            .unwrap_or(false);
        if let Some(save_button) = self.ivars().post_process_save_button.get() {
            save_button.setEnabled(enabled && dirty);
        }
    }

    fn refresh_post_process_models(&self) {
        let models = match crate::dictation::ollama::fetch_models() {
            Ok(models) => models,
            Err(err) => {
                self.set_post_process_status(&format!("Model refresh failed: {err}"), true);
                return;
            }
        };
        let Some(popup) = self.ivars().post_process_model_popup.get() else {
            return;
        };
        popup.removeAllItems();
        for model in &models {
            let title = NSString::from_str(model);
            popup.addItemWithTitle(&title);
        }
        if models.is_empty() {
            self.set_post_process_status("No models returned from Ollama.", true);
            return;
        }
        let current = crate::prefs::get_post_process_model();
        let selected = if models.iter().any(|m| m == &current) {
            current
        } else {
            models[0].clone()
        };
        let selected_title = NSString::from_str(&selected);
        popup.selectItemWithTitle(&selected_title);
        crate::prefs::set_post_process_model(&selected);
        self.set_post_process_status("", false);
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
    trigger_hub: Arc<Mutex<Sender<Trigger>>>,
    overlay_state: Arc<Mutex<OverlayState>>,
) -> UiHandles {
    let controller = AppController::new(mtm);

    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        install_main_menu(mtm);

        // Status item
        let status_bar = NSStatusBar::systemStatusBar();
        let status_item = status_bar.statusItemWithLength(VARIABLE_STATUS_ITEM_LENGTH);

        let icon_image = icon::build_template_icon();
        if let Some(button) = status_item.button(mtm) {
            button.setImage(Some(&icon_image));
            button.setToolTip(Some(ns_string!("hush — push-to-talk dictation")));
        }

        let menu = NSMenu::new(mtm);
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
        if let Some(prompt_text_view) = controller.ivars().post_process_prompt_text_view.get() {
            let prompt_text_view_obj: &AnyObject = prompt_text_view;
            center.addObserver_selector_name_object(
                observer,
                sel!(textDidChange:),
                Some(ns_string!("NSTextDidChangeNotification")),
                Some(prompt_text_view_obj),
            );
        }

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
    let _ = controller
        .ivars()
        .backend_switch_lock
        .set(Arc::new(Mutex::new(())));

    controller.refresh_perm_labels();
    controller.refresh_autostart();
    controller.refresh_backend();
    controller.refresh_post_process();
    controller.refresh_post_process_models();
    UiHandles { controller }
}

unsafe fn install_main_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let main_menu = NSMenu::new(mtm);
    let edit_root = NSMenuItem::new(mtm);
    edit_root.setTitle(ns_string!("Edit"));
    main_menu.addItem(&edit_root);

    let edit_menu = NSMenu::new(mtm);
    edit_menu.setTitle(ns_string!("Edit"));

    let undo_item = NSMenuItem::new(mtm);
    undo_item.setTitle(ns_string!("Undo"));
    undo_item.setAction(Some(sel!(undo:)));
    undo_item.setKeyEquivalent(ns_string!("z"));
    undo_item.setTarget(None);
    edit_menu.addItem(&undo_item);

    let redo_item = NSMenuItem::new(mtm);
    redo_item.setTitle(ns_string!("Redo"));
    redo_item.setAction(Some(sel!(redo:)));
    redo_item.setKeyEquivalent(ns_string!("Z"));
    redo_item.setTarget(None);
    edit_menu.addItem(&redo_item);

    edit_menu.addItem(&NSMenuItem::separatorItem(mtm));

    let cut_item = NSMenuItem::new(mtm);
    cut_item.setTitle(ns_string!("Cut"));
    cut_item.setAction(Some(sel!(cut:)));
    cut_item.setKeyEquivalent(ns_string!("x"));
    cut_item.setTarget(None);
    edit_menu.addItem(&cut_item);

    let copy_item = NSMenuItem::new(mtm);
    copy_item.setTitle(ns_string!("Copy"));
    copy_item.setAction(Some(sel!(copy:)));
    copy_item.setKeyEquivalent(ns_string!("c"));
    copy_item.setTarget(None);
    edit_menu.addItem(&copy_item);

    let paste_item = NSMenuItem::new(mtm);
    paste_item.setTitle(ns_string!("Paste"));
    paste_item.setAction(Some(sel!(paste:)));
    paste_item.setKeyEquivalent(ns_string!("v"));
    paste_item.setTarget(None);
    edit_menu.addItem(&paste_item);

    let select_all_item = NSMenuItem::new(mtm);
    select_all_item.setTitle(ns_string!("Select All"));
    select_all_item.setAction(Some(sel!(selectAll:)));
    select_all_item.setKeyEquivalent(ns_string!("a"));
    select_all_item.setTarget(None);
    edit_menu.addItem(&select_all_item);

    edit_root.setSubmenu(Some(&edit_menu));
    app.setMainMenu(Some(&main_menu));
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

unsafe fn build_settings_window(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(520.0, 680.0));

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
        ns_string!("Hold the fn key, speak, release to paste."),
        12.0,
        false,
    );
    subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
    stack.addArrangedSubview(&subtitle);

    let perms_heading = make_label(mtm, ns_string!("Permissions"), 14.0, true);
    stack.addArrangedSubview(&perms_heading);

    // Microphone — popup-style grant
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
    add_card(&stack, &mic_card);

    // Accessibility — System Settings. Gates BOTH the global fn-key
    // monitor (via NSEvent.addGlobalMonitor) and the Cmd+V paste
    // (via CGEventPost). Single perm covers both, no Input Monitoring.
    let acc_card = build_card(
        mtm,
        ns_string!("Accessibility"),
        ns_string!("Lets hush detect the fn key globally and paste the transcript by sending Cmd+V to the focused app."),
        ns_string!("Open Accessibility…"),
        sel!(grantAccessibility:),
        controller,
        |labels| {
            let _ = controller.ivars().accessibility_status_label.set(labels.status.clone());
            let _ = controller.ivars().accessibility_button.set(labels.button.clone());
        },
    );
    add_card(&stack, &acc_card);

    // Auto-start at login — backed by ~/Library/LaunchAgents/com.djmunro.hush.plist.
    let autostart_heading = make_label(mtm, ns_string!("General"), 14.0, true);
    stack.addArrangedSubview(&autostart_heading);

    let autostart_box = build_autostart_card(mtm, controller);
    add_card(&stack, &autostart_box);

    let transcription_heading = make_label(mtm, ns_string!("Transcription"), 14.0, true);
    stack.addArrangedSubview(&transcription_heading);

    let backend_box = build_backend_card(mtm, controller);
    add_card(&stack, &backend_box);

    let post_process_box = build_post_process_card(mtm, controller);
    add_card(&stack, &post_process_box);

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

unsafe fn build_backend_card(
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
    checkbox.setTitle(ns_string!("Use Parakeet TDT (parakeet-tdt-0.6b-v3)"));
    let target_obj: &AnyObject = controller;
    checkbox.setTarget(Some(target_obj));
    checkbox.setAction(Some(sel!(toggleBackend:)));
    inner.addArrangedSubview(&checkbox);

    let desc = make_label(
        mtm,
        ns_string!("NVIDIA's 0.6B ONNX model — downloads ~300 MB on first use. Switches live in the background."),
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

    let _ = controller.ivars().backend_checkbox.set(checkbox);

    box_view
}

unsafe fn build_post_process_card(
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

    let checkbox = NSButton::new(mtm);
    checkbox.setButtonType(objc2_app_kit::NSButtonType::Switch);
    checkbox.setTitle(ns_string!("Enable Ollama post-processing"));
    let target_obj: &AnyObject = controller;
    checkbox.setTarget(Some(target_obj));
    checkbox.setAction(Some(sel!(togglePostProcess:)));
    inner.addArrangedSubview(&checkbox);

    let row = NSStackView::new(mtm);
    row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    row.setSpacing(8.0);
    row.setDistribution(NSStackViewDistribution::Fill);
    row.setAlignment(NSLayoutAttribute::CenterY);
    row.setTranslatesAutoresizingMaskIntoConstraints(false);

    let popup = NSPopUpButton::new(mtm);
    popup.setTarget(Some(target_obj));
    popup.setAction(Some(sel!(selectPostProcessModel:)));
    row.addArrangedSubview(&popup);

    let refresh_button = NSButton::new(mtm);
    refresh_button.setTitle(ns_string!("Refresh"));
    refresh_button.setBezelStyle(NSBezelStyle::Rounded);
    refresh_button.setControlSize(NSControlSize::Regular);
    refresh_button.setTarget(Some(target_obj));
    refresh_button.setAction(Some(sel!(refreshPostProcessModels:)));
    row.addArrangedSubview(&refresh_button);

    inner.addArrangedSubview(&row);

    let desc = make_label(
        mtm,
        ns_string!(
            "Uses Ollama at http://localhost:11434/v1. Select a model and refresh to reload available models."
        ),
        11.0,
        false,
    );
    desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    desc.setUsesSingleLineMode(false);
    desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&desc);

    let prompt_desc = make_label(
        mtm,
        ns_string!(
            "Prompt controls how the model rewrites dictation before paste. Use ${output} to choose where the original transcript is inserted in the prompt."
        ),
        11.0,
        false,
    );
    prompt_desc.setTextColor(Some(&NSColor::secondaryLabelColor()));
    prompt_desc.setUsesSingleLineMode(false);
    prompt_desc.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    inner.addArrangedSubview(&prompt_desc);

    let prompt_scroll = NSScrollView::new(mtm);
    prompt_scroll.setHasVerticalScroller(true);
    prompt_scroll.setHasHorizontalScroller(false);
    prompt_scroll.setAutohidesScrollers(true);
    prompt_scroll.setTranslatesAutoresizingMaskIntoConstraints(false);

    let prompt_text_view = NSTextView::new(mtm);
    prompt_text_view.setUsesFindBar(false);
    prompt_text_view.setAllowsUndo(true);
    prompt_text_view.setEditable(true);
    prompt_text_view.setSelectable(true);
    prompt_text_view.setVerticallyResizable(true);
    prompt_text_view.setHorizontallyResizable(false);
    let current_prompt = NSString::from_str(&crate::prefs::get_post_process_prompt());
    prompt_text_view.setString(&current_prompt);
    prompt_scroll.setDocumentView(Some(&prompt_text_view));
    inner.addArrangedSubview(&prompt_scroll);

    let prompt_buttons = NSStackView::new(mtm);
    prompt_buttons.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    prompt_buttons.setSpacing(8.0);
    prompt_buttons.setDistribution(NSStackViewDistribution::Fill);
    prompt_buttons.setAlignment(NSLayoutAttribute::CenterY);
    prompt_buttons.setTranslatesAutoresizingMaskIntoConstraints(false);

    let save_button = NSButton::new(mtm);
    save_button.setTitle(ns_string!("Save"));
    save_button.setBezelStyle(NSBezelStyle::Rounded);
    save_button.setControlSize(NSControlSize::Regular);
    save_button.setTarget(Some(target_obj));
    save_button.setAction(Some(sel!(savePostProcessPrompt:)));
    save_button.setEnabled(false);
    prompt_buttons.addArrangedSubview(&save_button);

    let reset_button = NSButton::new(mtm);
    reset_button.setTitle(ns_string!("Reset"));
    reset_button.setBezelStyle(NSBezelStyle::Rounded);
    reset_button.setControlSize(NSControlSize::Regular);
    reset_button.setTarget(Some(target_obj));
    reset_button.setAction(Some(sel!(resetPostProcessPrompt:)));
    prompt_buttons.addArrangedSubview(&reset_button);

    inner.addArrangedSubview(&prompt_buttons);

    let status = make_label(mtm, ns_string!(""), 11.0, false);
    status.setTextColor(Some(&NSColor::secondaryLabelColor()));
    inner.addArrangedSubview(&status);

    box_view.setContentView(Some(&inner));

    let inner_view: &NSView = &inner;
    let box_super: &NSView = &box_view;
    pin_view_to_parent(inner_view, box_super);

    let row_view: &NSView = &row;
    row_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let desc_view: &NSView = &desc;
    desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let prompt_desc_view: &NSView = &prompt_desc;
    prompt_desc_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let prompt_scroll_view: &NSView = &prompt_scroll;
    prompt_scroll_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    let prompt_buttons_view: &NSView = &prompt_buttons;
    prompt_buttons_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&inner_view.widthAnchor(), -32.0)
        .setActive(true);

    // Keep long prompts to about 20 visible lines and use mouse-wheel scrolling.
    let prompt_max_height = prompt_scroll
        .heightAnchor()
        .constraintLessThanOrEqualToConstant(320.0);
    prompt_max_height.setActive(true);
    let prompt_min_height = prompt_scroll.heightAnchor().constraintEqualToConstant(320.0);
    prompt_min_height.setActive(true);

    let _ = controller.ivars().post_process_checkbox.set(checkbox);
    let _ = controller.ivars().post_process_model_popup.set(popup);
    let _ = controller
        .ivars()
        .post_process_prompt_text_view
        .set(prompt_text_view);
    let _ = controller.ivars().post_process_save_button.set(save_button);
    let _ = controller.ivars().post_process_status_label.set(status);

    box_view
}

/// Adds a card to the outer settings stack and pins its width so it
/// fills the available content area (the outer stack centers /
/// intrinsic-sizes its children otherwise).
unsafe fn add_card(stack: &NSStackView, card: &NSBox) {
    stack.addArrangedSubview(card);
    let card_view: &NSView = card;
    let stack_view: &NSView = stack;
    card_view
        .widthAnchor()
        .constraintEqualToAnchor_constant(&stack_view.widthAnchor(), -48.0)
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
