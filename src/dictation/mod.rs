//! Dictation pipeline: edge-triggered capture → transcribe → output, with
//! status events for the overlay. Hexagonal core (`pipeline`) + production
//! adapters wired by the `Dictation` facade.

pub mod cpal_capture;
pub mod output;
pub mod overlay_sink;
pub mod parakeet;
pub mod pipeline;
pub mod whisper;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub use pipeline::{StatusEvent, StatusSink, Trigger};

use pipeline::{Pipeline, Transcriber};

use crate::audio::Backend;
use crate::overlay::OverlayState;
use cpal_capture::CpalCapture;
use output::ClipboardPasteOutput;
use overlay_sink::OverlayStatusSink;
use parakeet::ParakeetTranscriber;
use whisper::WhisperTranscriber;

const MIN_SAMPLES: usize = 4_800;

pub struct Dictation {
    backend: Backend,
    overlay: Arc<Mutex<OverlayState>>,
}

impl Dictation {
    pub fn production(backend: Backend, overlay: Arc<Mutex<OverlayState>>) -> Self {
        Self { backend, overlay }
    }

    pub fn start_processing(self, rx: Receiver<Trigger>) {
        std::thread::spawn(move || {
            eprintln!("[hush] loading model…");
            let transcriber: Box<dyn Transcriber + Send + Sync> = match self.backend {
                Backend::Whisper(path) => Box::new(
                    WhisperTranscriber::new(&path).expect("load whisper model"),
                ),
                Backend::Parakeet(dir) => Box::new(
                    ParakeetTranscriber::new(&dir).expect("load parakeet model"),
                ),
            };
            let sink = OverlayStatusSink::new(self.overlay);
            let level_sink = sink.clone();
            let capture = CpalCapture::new(move |rms| {
                level_sink.publish(StatusEvent::LevelTick(rms));
            });
            let output = ClipboardPasteOutput;
            let pipeline = Pipeline::new(capture, transcriber, output, sink, MIN_SAMPLES);
            eprintln!("[hush] ready. hold fn to dictate.");
            pipeline.run(rx);
        });
    }
}
