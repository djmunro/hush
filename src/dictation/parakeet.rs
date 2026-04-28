use std::path::Path;
use std::sync::Mutex;

use parakeet_rs::Transcriber as _;
use parakeet_rs::ParakeetTDT;

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
        let model =
            ParakeetTDT::from_pretrained(dir, None).map_err(|e| format!("load parakeet: {e}"))?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(&self, pcm_16k: &[f32]) -> Result<String, String> {
        let mut model = self.model.lock().unwrap();
        let result = model
            .transcribe_samples(pcm_16k.to_vec(), 16_000, 1, None)
            .map_err(|e| format!("parakeet error: {e}"))?;
        Ok(result.text.trim().to_string())
    }
}
