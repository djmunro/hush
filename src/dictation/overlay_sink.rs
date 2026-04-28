use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use super::pipeline::{StatusEvent, StatusSink};
use crate::overlay::{OverlayMode, OverlayState};

const START_SOUND: &str = "/System/Library/Sounds/Tink.aiff";
const STOP_SOUND: &str = "/System/Library/Sounds/Pop.aiff";

#[derive(Clone)]
pub struct OverlayStatusSink {
    state: Arc<Mutex<OverlayState>>,
}

impl OverlayStatusSink {
    pub fn new(state: Arc<Mutex<OverlayState>>) -> Self {
        Self { state }
    }
}

impl StatusSink for OverlayStatusSink {
    fn publish(&self, ev: StatusEvent) {
        match ev {
            StatusEvent::Recording => {
                play(START_SOUND);
                OverlayState::set_mode(&self.state, OverlayMode::Recording);
            }
            StatusEvent::LevelTick(rms) => {
                OverlayState::push_level(&self.state, rms);
            }
            StatusEvent::Stopped => {
                play(STOP_SOUND);
            }
            StatusEvent::Transcribing => {
                OverlayState::set_mode(&self.state, OverlayMode::Transcribing);
            }
            StatusEvent::Idle => {
                OverlayState::set_mode(&self.state, OverlayMode::Hidden);
            }
            StatusEvent::Error(e) => {
                eprintln!("[hush] {e}");
            }
        }
    }
}

fn play(sound: &'static str) {
    if let Ok(mut child) = Command::new("afplay")
        .arg(sound)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
