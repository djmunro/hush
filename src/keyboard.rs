//! Native Cmd+V via CGEventPost — no osascript, no shellout, no
//! "python3.14 wants to send keystrokes" prompts. Permission attribution
//! lands on the hush binary itself.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

const KEYCODE_V: u16 = 9; // kVK_ANSI_V

pub fn paste(text: &str) -> Result<(), String> {
    let prev = Command::new("pbpaste")
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();

    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pbcopy: {e}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "pbcopy stdin".to_string())?
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    child.wait().map_err(|e| e.to_string())?;

    let post_result = post_cmd_v();

    // Give the receiving app a beat to consume the paste before we restore.
    std::thread::sleep(Duration::from_millis(150));

    if let Ok(mut c) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(stdin) = c.stdin.as_mut() {
            let _ = stdin.write_all(&prev);
        }
        let _ = c.wait();
    }

    post_result
}

fn post_cmd_v() -> Result<(), String> {
    let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "CGEventSource::new failed (Accessibility permission?)".to_string())?;

    let down = CGEvent::new_keyboard_event(src.clone(), KEYCODE_V, true)
        .map_err(|_| "CGEvent down failed".to_string())?;
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(src, KEYCODE_V, false)
        .map_err(|_| "CGEvent up failed".to_string())?;
    up.set_flags(CGEventFlags::CGEventFlagCommand);
    up.post(CGEventTapLocation::HID);

    Ok(())
}
