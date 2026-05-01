//! Dictation pipeline: edge-triggered capture → transcribe → output, with
//! status events for the overlay. Hexagonal core (`pipeline`) + production
//! adapters wired by the `Dictation` facade.

pub mod cpal_capture;
pub mod output;
pub mod overlay_sink;
pub mod parakeet;
pub mod parakeet_eou;
pub mod pipeline;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub use pipeline::{StatusEvent, StatusSink, Trigger};

use pipeline::{Pipeline, Transcriber};

use crate::audio;
use crate::overlay::OverlayState;
use cpal_capture::CpalCapture;
use output::ClipboardPasteOutput;
use overlay_sink::OverlayStatusSink;
use parakeet::ParakeetTranscriber;
use parakeet_eou::StreamingTranscriber;

const MIN_SAMPLES: usize = 4_800;

pub struct Dictation {
    streaming: bool,
    overlay: Arc<Mutex<OverlayState>>,
}

impl Dictation {
    pub fn production(streaming: bool, overlay: Arc<Mutex<OverlayState>>) -> Self {
        Self { streaming, overlay }
    }

    pub fn start_processing(self, rx: Receiver<Trigger>) {
        std::thread::spawn(move || {
            let model_path = audio::ensure_parakeet_model_path();
            eprintln!("[hush] loading model…");
            let transcriber: Box<dyn Transcriber + Send + Sync> = Box::new(
                ParakeetTranscriber::new(&model_path).expect("load parakeet model"),
            );
            let stream_transcriber: Option<StreamingTranscriber> = if self.streaming {
                let eou_path = audio::ensure_parakeet_eou_model_path();
                Some(
                    StreamingTranscriber::new(&eou_path).expect("load parakeet-eou model"),
                )
            } else {
                None
            };
            let sink = OverlayStatusSink::new(self.overlay);
            let level_sink = sink.clone();
            let capture = CpalCapture::new(move |rms| {
                level_sink.publish(StatusEvent::LevelTick(rms));
            });
            let output = ClipboardPasteOutput;
            let pipeline = Pipeline::new(
                capture,
                transcriber,
                stream_transcriber,
                output,
                sink,
                MIN_SAMPLES,
                self.streaming,
            );
            eprintln!("[hush] ready. hold fn to dictate.");
            pipeline.run(rx);
        });
    }
}
