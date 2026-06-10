use super::pipeline::Output;
use crate::config;
use crate::keyboard;
use crate::dictation::custom_parser;

pub struct ClipboardPasteOutput;

impl Output for ClipboardPasteOutput {
    fn deliver(&self, text: &str) -> Result<(), String> {
        let cfg = config::load();
        let text = custom_parser::apply(&cfg.custom_parser, text).unwrap_or_else(|| text.to_string());
        keyboard::paste(&text)
    }
}
