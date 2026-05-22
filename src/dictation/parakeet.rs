use std::path::Path;
use std::sync::Mutex;

use parakeet_rs::Transcriber as _;
use parakeet_rs::{ParakeetTDT, ExecutionConfig, ExecutionProvider};

use super::pipeline::Transcriber;

pub struct ParakeetTranscriber {
    // ParakeetTDT::transcribe_samples takes &mut self, so we need interior mutability.
    model: Mutex<ParakeetTDT>,
}

impl ParakeetTranscriber {
    pub fn new(model_dir: &Path) -> Result<Self, String> {
        let dir = model_dir
            .to_str()
            .ok_or_else(|| "non-utf8 model path".to_string())?;
        
        // For Apple Silicon / macOS Sequoia compatibility, CoreML is unstable 
        // with the TDT models (especially 1.1B) and causes exceptions during initialization.
        // Forcing CPU Execution Provider avoids these crashes and is still exceptionally fast.
        let config = ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cpu);
        let model =
            ParakeetTDT::from_pretrained(dir, Some(config)).map_err(|e| format!("load parakeet: {e}"))?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(&self, pcm_16k: &[f32]) -> Result<String, String> {
        eprintln!("[hush] starting transcription of {} samples ({:.2}s of audio)…", pcm_16k.len(), pcm_16k.len() as f32 / 16000.0);
        let start = std::time::Instant::now();
        let mut model = self.model.lock().unwrap();
        
        // Use Some(parakeet_rs::TimestampMode::Sentences) for TDT models to ensure correct segmentation and punctuation.
        let mode = Some(parakeet_rs::TimestampMode::Sentences);
        
        let result = model
            .transcribe_samples(pcm_16k.to_vec(), 16_000, 1, mode)
            .map_err(|e| {
                let err_msg = format!("parakeet error: {e}");
                eprintln!("[hush] transcription failed after {:.2?}: {}", start.elapsed(), err_msg);
                err_msg
            })?;
        
        let duration = start.elapsed();
        let text = result.text.trim().to_string();
        eprintln!(
            "[hush] transcription completed in {:.2?} (RTF: {:.3}x). Result: \"{}\"",
            duration,
            duration.as_secs_f32() / (pcm_16k.len() as f32 / 16000.0),
            text
        );
        Ok(text)
    }
}
