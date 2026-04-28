use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::pipeline::Transcriber;

const DEFAULT_PROMPT: &str = "Dictation with proper punctuation and capitalization. \
Technical terms: TypeScript, JavaScript, Python, Rust, Go, GitHub, npm, API, JSON, \
HTTP, SQL, GraphQL, AWS, Docker, Kubernetes, macOS, async, await.";

pub struct WhisperTranscriber {
    ctx: WhisperContext,
    prompt: String,
}

impl WhisperTranscriber {
    pub fn new(model_path: &Path) -> Result<Self, String> {
        let path = model_path
            .to_str()
            .ok_or_else(|| "non-utf8 model path".to_string())?;
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("load whisper: {e}"))?;
        let prompt = std::env::var("WHISPER_PROMPT").unwrap_or_else(|_| DEFAULT_PROMPT.to_string());
        Ok(Self { ctx, prompt })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, pcm_16k: &[f32]) -> Result<String, String> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("state error: {e}"))?;
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: 1.0,
        });
        params.set_language(Some("en"));
        params.set_no_context(true);
        params.set_initial_prompt(&self.prompt);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.2);
        params.set_no_speech_thold(0.6);
        params.set_suppress_blank(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        state
            .full(params, pcm_16k)
            .map_err(|e| format!("transcribe error: {e}"))?;

        let n = state.full_n_segments();
        let mut text = String::new();
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                if let Ok(s) = seg.to_str_lossy() {
                    text.push_str(&s);
                }
            }
        }
        Ok(text.trim().to_string())
    }
}
