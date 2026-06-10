use std::sync::Mutex;

use super::pipeline::Output;
use crate::config;
use crate::keyboard;
use crate::dictation::custom_parser;

#[derive(Default)]
pub struct ClipboardPasteOutput {
    last_pasted: Mutex<String>,
}

impl Output for ClipboardPasteOutput {
    fn deliver(&self, text: &str) -> Result<(), String> {
        let cfg = config::load();
        let prev = self.last_pasted.lock().unwrap().clone();
        let text =
            custom_parser::apply(&cfg.custom_parser, text, &prev).unwrap_or_else(|| text.to_string());
        keyboard::paste(&text)?;
        *self.last_pasted.lock().unwrap() = text;
        Ok(())
    }
}
