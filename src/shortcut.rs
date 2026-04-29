//! Global shortcut monitor.
//!
//! Watches `FlagsChanged | KeyDown | KeyUp` via NSEvent.addGlobalMonitor.
//! Same trick as install_fn_monitor — gates only on Accessibility, never
//! Input Monitoring.
//!
//! State machine: track which configured modifiers are currently held and
//! (if the binding has a non-mod key) whether that key is held. When *all*
//! parts are held → Trigger::Start. When any drops → Trigger::Stop.
//!
//! Recording mode: a parallel callback receives raw events while the
//! settings UI is "press a shortcut" — captures whatever the user presses
//! and hands back a Shortcut.

use std::ptr::NonNull;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};

use crate::config::{ModKey, Shortcut};
use crate::dictation::Trigger;

// Virtual keycodes for the modifier keys (kVK_* from Carbon's Events.h).
// These are the only reliable way to distinguish left vs right modifier
// presses — NSEventModifierFlags bits don't differentiate sides.
const KVK_COMMAND: u16 = 0x37;
const KVK_RIGHT_COMMAND: u16 = 0x36;
const KVK_OPTION: u16 = 0x3A;
const KVK_RIGHT_OPTION: u16 = 0x3D;
const KVK_CONTROL: u16 = 0x3B;
const KVK_RIGHT_CONTROL: u16 = 0x3E;
const KVK_SHIFT: u16 = 0x38;
const KVK_RIGHT_SHIFT: u16 = 0x3C;
const KVK_CAPS_LOCK: u16 = 0x39;
const KVK_FUNCTION: u16 = 0x3F;

fn keycode_to_modkey(code: u16) -> Option<ModKey> {
    Some(match code {
        KVK_COMMAND => ModKey::LeftCommand,
        KVK_RIGHT_COMMAND => ModKey::RightCommand,
        KVK_OPTION => ModKey::LeftOption,
        KVK_RIGHT_OPTION => ModKey::RightOption,
        KVK_CONTROL => ModKey::LeftControl,
        KVK_RIGHT_CONTROL => ModKey::RightControl,
        KVK_SHIFT => ModKey::LeftShift,
        KVK_RIGHT_SHIFT => ModKey::RightShift,
        KVK_CAPS_LOCK => ModKey::CapsLock,
        KVK_FUNCTION => ModKey::Fn,
        _ => return None,
    })
}

fn modkey_flag_bit(m: ModKey) -> Option<NSEventModifierFlags> {
    Some(match m {
        ModKey::Fn => NSEventModifierFlags::Function,
        ModKey::LeftCommand | ModKey::RightCommand => NSEventModifierFlags::Command,
        ModKey::LeftOption | ModKey::RightOption => NSEventModifierFlags::Option,
        ModKey::LeftControl | ModKey::RightControl => NSEventModifierFlags::Control,
        ModKey::LeftShift | ModKey::RightShift => NSEventModifierFlags::Shift,
        ModKey::CapsLock => NSEventModifierFlags::CapsLock,
    })
}

#[derive(Default)]
struct MonitorState {
    held_mods: Vec<ModKey>,
    key_down: bool,
    chord_active: bool,
}

type RecorderCb = Box<dyn FnMut(Option<Shortcut>) + Send>;

struct SharedState {
    binding: Shortcut,
    monitor_state: MonitorState,
    session: SessionMods,
    recording_buf: RecordingBuf,
    recorder: Option<RecorderCb>,
}

pub struct ShortcutMonitor {
    inner: Arc<Mutex<SharedState>>,
    _global: Retained<AnyObject>,
    _local: Retained<AnyObject>,
}

impl ShortcutMonitor {
    pub fn install(initial: Shortcut, tx: Sender<Trigger>) -> Option<Self> {
        let inner = Arc::new(Mutex::new(SharedState {
            binding: initial,
            monitor_state: MonitorState::default(),
            session: SessionMods::default(),
            recording_buf: RecordingBuf::default(),
            recorder: None,
        }));

        // Global monitor: events that target *other* apps (push-to-talk
        // path). Returns nothing — observer-only.
        let inner_g = inner.clone();
        let tx_g = tx.clone();
        let global_handler = block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| {
            let event = unsafe { event_ptr.as_ref() };
            handle_event(&inner_g, &tx_g, event);
        });
        let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::FlagsChanged | NSEventMask::KeyDown | NSEventMask::KeyUp,
            &global_handler,
        )?;

        // Local monitor: events that target hush itself (Settings window
        // focused). Required because addGlobalMonitor does NOT fire when
        // hush is the active app — without this, the recording UI sees
        // nothing the moment the user clicks Record. Returning Some(event)
        // forwards it to the responder chain; returning None swallows it.
        let inner_l = inner.clone();
        let tx_l = tx;
        let local_handler =
            block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
                let event = unsafe { event_ptr.as_ref() };
                let recording = inner_l.lock().unwrap().recorder.is_some();
                handle_event(&inner_l, &tx_l, event);
                if recording {
                    // Swallow so Esc doesn't close the window, Space doesn't
                    // toggle a button, etc.
                    std::ptr::null_mut()
                } else {
                    event_ptr.as_ptr()
                }
            });
        let local = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::FlagsChanged | NSEventMask::KeyDown | NSEventMask::KeyUp,
                &local_handler,
            )
        }?;

        Some(Self {
            inner,
            _global: global,
            _local: local,
        })
    }

    pub fn set_binding(&self, new: Shortcut) {
        let mut st = self.inner.lock().unwrap();
        st.binding = new;
        st.monitor_state.chord_active = false;
        st.monitor_state.key_down = false;
    }

    pub fn start_recording<F: FnMut(Option<Shortcut>) + Send + 'static>(&self, cb: F) {
        let mut st = self.inner.lock().unwrap();
        st.recording_buf = RecordingBuf::default();
        // Don't reset `session` — the user may already be holding a
        // modifier when they click Record (their last click released the
        // mouse button, but if they're chording, mods are still down).
        st.recorder = Some(Box::new(cb));
    }
}

fn handle_event(inner: &Arc<Mutex<SharedState>>, tx: &Sender<Trigger>, event: &NSEvent) {
    let etype = event.r#type();
    let flags = event.modifierFlags();
    let code = if matches!(
        etype,
        NSEventType::FlagsChanged | NSEventType::KeyDown | NSEventType::KeyUp
    ) {
        event.keyCode()
    } else {
        return;
    };

    let mut st = inner.lock().unwrap();

    if etype == NSEventType::FlagsChanged {
        if let Some(mk) = keycode_to_modkey(code) {
            let held_now = is_modkey_pressed(mk, flags, code, &st.session);
            st.session.set(mk, held_now);
        }
    }

    // Recording path takes priority and short-circuits.
    if st.recorder.is_some() {
        // Esc cancels.
        if etype == NSEventType::KeyDown && code == 0x35 {
            let cb = st.recorder.take();
            st.recording_buf = RecordingBuf::default();
            drop(st);
            if let Some(mut cb) = cb {
                cb(None);
            }
            return;
        }

        let sess_snapshot = st.session;
        let label = if etype == NSEventType::KeyDown {
            Some(keycode_label(code, event))
        } else {
            None
        };
        let captured = st
            .recording_buf
            .feed(etype, code, label, sess_snapshot);
        if let Some(shortcut) = captured {
            let cb = st.recorder.take();
            st.recording_buf = RecordingBuf::default();
            drop(st);
            if let Some(mut cb) = cb {
                cb(Some(shortcut));
            }
        }
        return;
    }

    // Push-to-talk evaluation.
    let bind = st.binding.clone();
    st.monitor_state.held_mods = st.session.held();

    if let Some(want_code) = bind.key {
        if etype == NSEventType::KeyDown && code == want_code {
            st.monitor_state.key_down = true;
        } else if etype == NSEventType::KeyUp && code == want_code {
            st.monitor_state.key_down = false;
        }
    }

    let should_be_active = bind_satisfied(&bind, &st.monitor_state);
    if should_be_active && !st.monitor_state.chord_active {
        st.monitor_state.chord_active = true;
        let _ = tx.send(Trigger::Start);
    } else if !should_be_active && st.monitor_state.chord_active {
        st.monitor_state.chord_active = false;
        let _ = tx.send(Trigger::Stop);
    }
}

fn bind_satisfied(bind: &Shortcut, st: &MonitorState) -> bool {
    if bind.mods.is_empty() && bind.key.is_none() {
        return false;
    }
    for m in &bind.mods {
        if !st.held_mods.contains(m) {
            return false;
        }
    }
    if bind.key.is_some() && !st.key_down {
        return false;
    }
    true
}

fn is_modkey_pressed(
    mk: ModKey,
    flags: NSEventModifierFlags,
    code: u16,
    sess: &SessionMods,
) -> bool {
    // For sided modifiers, the flag bit only tells us "some side is down."
    // We disambiguate: if the keycode of *this* event matches the side,
    // then the press/release applies to that side. Otherwise consult the
    // existing session.
    let bit = match modkey_flag_bit(mk) {
        Some(b) => b,
        None => return false,
    };
    let bit_set = flags.contains(bit);

    let event_is_this_side = keycode_to_modkey(code) == Some(mk);
    if event_is_this_side {
        // Press if bit became set, release if it cleared.
        return bit_set;
    }
    // Event was for the *other* side of the same modifier (or unrelated).
    // Preserve our previous belief unless the bit cleared entirely.
    if !bit_set {
        return false;
    }
    sess.contains(mk)
}

#[derive(Default, Clone, Copy)]
struct SessionMods {
    fn_: bool,
    lcmd: bool,
    rcmd: bool,
    lopt: bool,
    ropt: bool,
    lctrl: bool,
    rctrl: bool,
    lshift: bool,
    rshift: bool,
    caps: bool,
}

impl SessionMods {
    fn set(&mut self, mk: ModKey, v: bool) {
        match mk {
            ModKey::Fn => self.fn_ = v,
            ModKey::LeftCommand => self.lcmd = v,
            ModKey::RightCommand => self.rcmd = v,
            ModKey::LeftOption => self.lopt = v,
            ModKey::RightOption => self.ropt = v,
            ModKey::LeftControl => self.lctrl = v,
            ModKey::RightControl => self.rctrl = v,
            ModKey::LeftShift => self.lshift = v,
            ModKey::RightShift => self.rshift = v,
            ModKey::CapsLock => self.caps = v,
        }
    }
    fn contains(&self, mk: ModKey) -> bool {
        match mk {
            ModKey::Fn => self.fn_,
            ModKey::LeftCommand => self.lcmd,
            ModKey::RightCommand => self.rcmd,
            ModKey::LeftOption => self.lopt,
            ModKey::RightOption => self.ropt,
            ModKey::LeftControl => self.lctrl,
            ModKey::RightControl => self.rctrl,
            ModKey::LeftShift => self.lshift,
            ModKey::RightShift => self.rshift,
            ModKey::CapsLock => self.caps,
        }
    }
    fn held(&self) -> Vec<ModKey> {
        let mut v = Vec::new();
        if self.fn_ {
            v.push(ModKey::Fn);
        }
        if self.lcmd {
            v.push(ModKey::LeftCommand);
        }
        if self.rcmd {
            v.push(ModKey::RightCommand);
        }
        if self.lopt {
            v.push(ModKey::LeftOption);
        }
        if self.ropt {
            v.push(ModKey::RightOption);
        }
        if self.lctrl {
            v.push(ModKey::LeftControl);
        }
        if self.rctrl {
            v.push(ModKey::RightControl);
        }
        if self.lshift {
            v.push(ModKey::LeftShift);
        }
        if self.rshift {
            v.push(ModKey::RightShift);
        }
        if self.caps {
            v.push(ModKey::CapsLock);
        }
        v
    }
}

/// Recording state machine. Accumulates every modifier and (at most one)
/// non-modifier key the user touches, then finalizes when *everything* is
/// released. Lets the user roll fingers — press cmd, then opt, release cmd
/// while still holding opt — and only commits on full release.
#[derive(Default)]
struct RecordingBuf {
    pending_mods: Vec<ModKey>,
    pending_key: Option<u16>,
    pending_key_label: Option<String>,
    non_mod_down: bool,
    saw_any_press: bool,
}

impl RecordingBuf {
    fn feed(
        &mut self,
        etype: NSEventType,
        code: u16,
        key_label: Option<String>,
        sess: SessionMods,
    ) -> Option<Shortcut> {
        let held = sess.held();
        if !held.is_empty() {
            self.saw_any_press = true;
            for m in &held {
                if !self.pending_mods.contains(m) {
                    self.pending_mods.push(*m);
                }
            }
        }

        if etype == NSEventType::KeyDown {
            if self.pending_key.is_none() {
                self.pending_key = Some(code);
                self.pending_key_label = key_label;
            }
            self.non_mod_down = true;
            self.saw_any_press = true;
        } else if etype == NSEventType::KeyUp {
            self.non_mod_down = false;
        }

        if self.saw_any_press && held.is_empty() && !self.non_mod_down {
            let mods = std::mem::take(&mut self.pending_mods);
            let key = self.pending_key.take();
            let label = self.pending_key_label.take();
            if mods.is_empty() && key.is_none() {
                return None;
            }
            return Some(Shortcut {
                mods,
                key,
                key_label: label,
            });
        }

        None
    }
}

fn keycode_label(code: u16, event: &NSEvent) -> String {
    // Try to pull the typed character (respects layout). Fall back to
    // "key0xNN" if the OS gives us nothing printable.
    let chars = event.charactersIgnoringModifiers();
    if let Some(s) = chars {
        let raw = s.to_string();
        if !raw.is_empty() {
            let upper = raw.to_uppercase();
            // Filter out whitespace / non-printable garbage.
            if upper.chars().all(|c| !c.is_control()) {
                return special_name(&upper, code).unwrap_or(upper);
            }
        }
    }
    format!("key0x{code:02X}")
}

fn special_name(label: &str, code: u16) -> Option<String> {
    // Map common non-printable / ambiguous characters to readable names.
    match code {
        0x31 => Some("Space".into()),
        0x24 => Some("Return".into()),
        0x30 => Some("Tab".into()),
        0x35 => Some("Esc".into()),
        0x33 => Some("Delete".into()),
        0x7B => Some("←".into()),
        0x7C => Some("→".into()),
        0x7D => Some("↓".into()),
        0x7E => Some("↑".into()),
        _ => {
            if label == " " {
                Some("Space".into())
            } else {
                None
            }
        }
    }
}
