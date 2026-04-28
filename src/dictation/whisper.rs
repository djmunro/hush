use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::pipeline::Transcriber;

pub struct WhisperTranscriber {
    ctx: WhisperContext,
}

impl WhisperTranscriber {
    pub fn new(model_path: &Path) -> Result<Self, String> {
        let path = model_path
            .to_str()
            .ok_or_else(|| "non-utf8 model path".to_string())?;
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("load whisper: {e}"))?;
        Ok(Self { ctx })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, pcm_16k: &[f32]) -> Result<String, String> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("state error: {e}"))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_no_context(true);
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
