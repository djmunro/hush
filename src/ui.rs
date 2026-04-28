//! Menubar status item + settings window. Built on objc2/AppKit.

#![allow(deprecated)]

use std::cell::{Cell, OnceCell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBezelStyle, NSBox,
    NSBoxType, NSButton, NSColor, NSControlSize, NSControlStateValueOff, NSControlStateValueOn,
    NSFont, NSLayoutAttribute, NSLineBreakMode, NSMenu, NSMenuItem, NSStackView,
    NSStackViewDistribution, NSStatusBar, NSStatusItem, NSTextField,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    ns_string, MainThreadMarker, NSEdgeInsets, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};

use crate::autostart;
use crate::icon;
use crate::perms::{self, MicState, PermStatus};
use crate::TapHandle;

const VARIABLE_STATUS_ITEM_LENGTH: CGFloat = -1.0;

#[derive(Default)]
pub struct ControllerIvars {
    status_item: OnceCell<Retained<NSStatusItem>>,
    settings_window: OnceCell<Retained<NSWindow>>,
    mic_status_label: OnceCell<Retained<NSTextField>>,
    mic_button: OnceCell<Retained<NSButton>>,
    input_status_label: OnceCell<Retained<NSTextField>>,
    input_button: OnceCell<Retained<NSButton>>,
    input_waiting: Cell<bool>,
    input_wait_timer: RefCell<Option<Retained<NSTimer>>>,
    accessibility_status_label: OnceCell<Retained<NSTextField>>,
    accessibility_button: OnceCell<Retained<NSButton>>,
    accessibility_waiting: Cell<bool>,
    accessibility_wait_timer: RefCell<Option<Retained<NSTimer>>>,
    autostart_checkbox: OnceCell<Retained<NSButton>>,
    tap_handle: OnceCell<TapHandle>,
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

        #[unsafe(method(grantInputMonitoring:))]
        fn grant_input_monitoring(&self, _sender: Option<&AnyObject>) {
            // Force-register the binary in TCC's Input Monitoring
            // list so the user has something to toggle in Settings.
            // We deliberately do NOT call try_install here: every
            // CGEventTapCreate against an unauthorized cdhash
            // re-fires the TCC prompt. The polling tick will install
            // exactly once, after the perm flips to granted.
            perms::request_input_monitoring();
            perms::open_input_monitoring_pane();
            self.start_input_wait();
        }

        #[unsafe(method(grantAccessibility:))]
        fn grant_accessibility(&self, _sender: Option<&AnyObject>) {
            perms::request_accessibility();
            perms::open_accessibility_pane();
            self.start_accessibility_wait();
        }

        #[unsafe(method(inputWaitTimeout:))]
        fn input_wait_timeout(&self, _timer: Option<&AnyObject>) {
            self.ivars().input_waiting.set(false);
            self.ivars().input_wait_timer.replace(None);
            self.refresh_perm_labels();
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

    fn refresh_perm_labels(&self) {
        let status = PermStatus::check();
        let mic_granted = status.mic_granted();

        // If a perm flipped to granted while we were in the
        // "Waiting…" window, clear the wait so the UI reflects
        // the grant immediately and the timeout doesn't fire
        // a redundant refresh.
        if status.input_monitoring && self.ivars().input_waiting.get() {
            self.ivars().input_waiting.set(false);
            if let Some(t) = self.ivars().input_wait_timer.replace(None) {
                t.invalidate();
            }
        }
        if status.accessibility && self.ivars().accessibility_waiting.get() {
            self.ivars().accessibility_waiting.set(false);
            if let Some(t) = self.ivars().accessibility_wait_timer.replace(None) {
                t.invalidate();
            }
        }

        // Lazily install the event tap exactly once, the moment
        // Input Monitoring becomes granted. TapHandle::try_install
        // consumes its sender on attempt, so this is safe to call
        // every refresh — subsequent calls are a no-op.
        if status.input_monitoring {
            if let Some(handle) = self.ivars().tap_handle.get() {
                handle.try_install();
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
            if let Some(label) = self.ivars().input_status_label.get() {
                label.setStringValue(&perm_status_text(status.input_monitoring));
                label.setTextColor(Some(&perm_color(status.input_monitoring)));
            }
            if let Some(button) = self.ivars().input_button.get() {
                self.apply_grant_button(
                    button,
                    status.input_monitoring,
                    self.ivars().input_waiting.get(),
                    ns_string!("Open Input Monitoring…"),
                );
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

    fn start_input_wait(&self) {
        self.ivars().input_waiting.set(true);
        let timer = unsafe {
            let target: &AnyObject = self;
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                10.0,
                target,
                sel!(inputWaitTimeout:),
                None,
                false,
            )
        };
        if let Some(prev) = self.ivars().input_wait_timer.replace(Some(timer)) {
            prev.invalidate();
        }
        self.refresh_perm_labels();
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

pub fn install_menubar_and_window(mtm: MainThreadMarker, tap_handle: TapHandle) -> UiHandles {
    let controller = AppController::new(mtm);
    let _ = controller.ivars().tap_handle.set(tap_handle);

    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

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

    controller.refresh_perm_labels();
    controller.refresh_autostart();
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

unsafe fn build_settings_window(
    mtm: MainThreadMarker,
    controller: &AppController,
) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(520.0, 540.0));

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

    // Input Monitoring — System Settings
    let input_card = build_card(
        mtm,
        ns_string!("Input Monitoring"),
        ns_string!("Lets hush detect when you press and release the fn key. macOS only grants this from System Settings."),
        ns_string!("Open Input Monitoring…"),
        sel!(grantInputMonitoring:),
        controller,
        |labels| {
            let _ = controller.ivars().input_status_label.set(labels.status.clone());
            let _ = controller.ivars().input_button.set(labels.button.clone());
        },
    );
    add_card(&stack, &input_card);

    // Accessibility — System Settings
    let acc_card = build_card(
        mtm,
        ns_string!("Accessibility"),
        ns_string!("Lets hush paste the transcript by sending Cmd+V to the focused app. Requires System Settings."),
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
