use super::pipeline::Output;
use crate::keyboard;

pub struct ClipboardPasteOutput;

impl Output for ClipboardPasteOutput {
    fn deliver(&self, text: &str) -> Result<(), String> {
        keyboard::paste(text)
    }
}
