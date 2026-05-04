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

use crate::audio;
use crate::config::{BackendKind, Config};
use crate::overlay::OverlayState;
use cpal_capture::CpalCapture;
use output::ClipboardPasteOutput;
use overlay_sink::OverlayStatusSink;
use parakeet::ParakeetTranscriber;
use whisper::WhisperTranscriber;

const MIN_SAMPLES: usize = 4_800;

pub struct Dictation {
    kind: BackendKind,
    overlay: Arc<Mutex<OverlayState>>,
}

impl Dictation {
    pub fn production(config: &Config, overlay: Arc<Mutex<OverlayState>>) -> Self {
        Self {
            kind: config.backend,
            overlay,
        }
    }

    pub fn start_processing(self, rx: Receiver<Trigger>) {
        std::thread::spawn(move || {
            // Bootstrap (download if missing) and load on the worker thread so
            // first-launch and live-toggle never block main / NSApp.
            let model_path = audio::ensure_model_for(self.kind);
            eprintln!("[hush] loading model…");
            let transcriber: Box<dyn Transcriber + Send + Sync> = match self.kind {
                BackendKind::Whisper => Box::new(
                    WhisperTranscriber::new(&model_path).expect("load whisper model"),
                ),
                BackendKind::Parakeet => Box::new(
                    ParakeetTranscriber::new(&model_path).expect("load parakeet model"),
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
