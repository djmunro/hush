use parakeet_rs::{ParakeetEOU, ParakeetEOUHandle};
use std::path::Path;

use super::pipeline::StreamTranscriber;

/// Streaming transcriber backed by ParakeetEOU (120M realtime EOU model).
///
/// The ONNX session is loaded once via `ParakeetEOUHandle` at construction time.
/// Each recording gets a fresh `ParakeetEOU` instance (`from_shared`) so decoder
/// state never bleeds across utterances.  `feed` returns partial text decoded for
/// that 160ms chunk so the pipeline can paste words as they arrive.
///
/// Note: the model requires ~1 second of audio before producing any output.
/// Very short utterances (<1 s) will produce empty strings from all feed calls;
/// the pipeline's min_samples guard then skips the flush.
pub struct StreamingTranscriber {
    handle: ParakeetEOUHandle,
    eou: Option<ParakeetEOU>,
    total_fed: usize,
}

impl StreamingTranscriber {
    pub fn new(model_dir: &Path) -> Result<Self, String> {
        let handle = ParakeetEOUHandle::load(model_dir, None)
            .map_err(|e| format!("load parakeet-eou: {}", e))?;
        Ok(Self { handle, eou: None, total_fed: 0 })
    }
}

impl StreamTranscriber for StreamingTranscriber {
    fn start_session(&mut self) -> Result<(), String> {
        self.eou = Some(ParakeetEOU::from_shared(&self.handle));
        self.total_fed = 0;
        Ok(())
    }

    fn feed(&mut self, chunk: &[f32]) -> Result<String, String> {
        self.total_fed += chunk.len();
        let eou = self.eou.as_mut().ok_or("no active session")?;
        eou.transcribe(chunk, false)
            .map_err(|e| format!("parakeet-eou: {}", e))
    }

    fn flush(&mut self, remaining: &[f32]) -> Result<String, String> {
        let eou = self.eou.as_mut().ok_or("no active session")?;
        eou.transcribe(remaining, false)
            .map_err(|e| format!("parakeet-eou flush: {}", e))
    }

    fn sample_count(&self) -> usize {
        self.total_fed
    }
}
