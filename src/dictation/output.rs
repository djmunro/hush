use super::pipeline::Output;
use crate::config;
use crate::keyboard;

pub struct ClipboardPasteOutput;

impl Output for ClipboardPasteOutput {
    fn deliver(&self, text: &str) -> Result<(), String> {
        let cleanup = config::load().cleanup;
        keyboard::paste(text, &cleanup)
    }
}
