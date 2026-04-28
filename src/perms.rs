//! macOS permission probes + grant helpers.
//!
//! Two of the three perms (Input Monitoring, Accessibility) require a
//! trip to System Settings — the OS-level prompts can't be re-shown
//! reliably once dismissed once. Microphone, however, has a real
//! requestAccess API that pops the standard system dialog right inside
//! the app. We use that for a one-click grant.

use std::process::Command;

use block2::RcBlock;
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicState {
    NotDetermined,
    Granted,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermStatus {
    pub microphone: MicState,
    pub input_monitoring: bool,
    pub accessibility: bool,
}

impl PermStatus {
    pub fn check() -> Self {
        unsafe {
            let mic_state = match AVMediaTypeAudio {
                Some(t) => match AVCaptureDevice::authorizationStatusForMediaType(t) {
                    AVAuthorizationStatus::Authorized => MicState::Granted,
                    AVAuthorizationStatus::NotDetermined => MicState::NotDetermined,
                    _ => MicState::Denied,
                },
                None => MicState::NotDetermined,
            };
            Self {
                microphone: mic_state,
                input_monitoring: CGPreflightListenEventAccess(),
                accessibility: AXIsProcessTrusted(),
            }
        }
    }

    pub fn mic_granted(&self) -> bool {
        matches!(self.microphone, MicState::Granted)
    }

    pub fn all_granted(&self) -> bool {
        self.mic_granted() && self.input_monitoring && self.accessibility
    }
}

pub fn request_input_monitoring() {
    unsafe {
        CGRequestListenEventAccess();
    }
}

/// Triggers the canonical macOS Accessibility prompt and registers the
/// running binary in the Accessibility list. This is the API that
/// actually makes the app appear in System Settings → Privacy &
/// Security → Accessibility — `CGRequestPostEventAccess` can silently
/// no-op until the binary tries to post an event.
pub fn request_accessibility() {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = CFBoolean::true_value();
        let opts = CFDictionary::from_CFType_pairs(&[(key, value)]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef());
    }
}

/// Pops the standard system "would like to access the microphone"
/// dialog. Async — the result fires on an arbitrary dispatch queue;
/// `on_done` is invoked from there with the final granted/denied state.
pub fn request_microphone(on_done: impl Fn(bool) + Send + Sync + 'static) {
    unsafe {
        let media_type = match AVMediaTypeAudio {
            Some(t) => t,
            None => {
                on_done(false);
                return;
            }
        };
        let block = RcBlock::new(move |granted: Bool| {
            on_done(granted.as_bool());
        });
        AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &block);
    }
}

pub fn open_input_monitoring_pane() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
        .status();
}

pub fn open_accessibility_pane() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .status();
}

pub fn open_microphone_pane() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .status();
}
