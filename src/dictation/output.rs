use super::pipeline::Output;
use crate::dictation::ollama;
use crate::keyboard;
use crate::prefs;

pub struct ClipboardPasteOutput;

impl Output for ClipboardPasteOutput {
    fn deliver(&self, text: &str) -> Result<(), String> {
        keyboard::paste(text)
    }
}

pub struct PostProcessOutput<O> {
    inner: O,
}

impl<O> PostProcessOutput<O> {
    pub fn new(inner: O) -> Self {
        Self { inner }
    }
}

impl<O: Output> Output for PostProcessOutput<O> {
    fn deliver(&self, text: &str) -> Result<(), String> {
        let final_text = if prefs::get_post_process_enabled() {
            let model = prefs::get_post_process_model();
            let prompt = prefs::get_post_process_prompt();
            match ollama::post_process(&model, &prompt, text) {
                Ok(refined) => refined,
                Err(err) => {
                    eprintln!("[hush] post-process failed ({model}): {err}");
                    text.to_string()
                }
            }
        } else {
            text.to_string()
        };
        self.inner.deliver(&final_text)
    }
}
