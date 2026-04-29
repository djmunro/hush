//! Synchronous pipeline state machine + four port traits. No threads, no
//! timers, no AppKit. Driven by `handle(Trigger)`; `run(rx)` is a thin
//! receive loop that production wiring uses.

use std::sync::mpsc::Receiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Start,
    Stop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatusEvent {
    Recording,
    LevelTick(f32),
    Stopped,
    Transcribing,
    Idle,
    Error(String),
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum PipelineError {
    Capture(String),
    Transcribe(String),
    Output(String),
}

pub trait Capture: Send {
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self) -> Result<Vec<f32>, String>;
}

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, pcm_16k: &[f32]) -> Result<String, String>;
}

impl<T: Transcriber + ?Sized> Transcriber for Box<T> {
    fn transcribe(&self, pcm_16k: &[f32]) -> Result<String, String> {
        (**self).transcribe(pcm_16k)
    }
}

pub trait Output: Send + Sync {
    fn deliver(&self, text: &str) -> Result<(), String>;
}

pub trait StatusSink: Send + Sync + Clone {
    fn publish(&self, ev: StatusEvent);
}

pub struct Pipeline<C, T, O, S> {
    capture: C,
    transcriber: T,
    output: O,
    sink: S,
    min_samples: usize,
    recording: bool,
}

impl<C: Capture, T: Transcriber, O: Output, S: StatusSink> Pipeline<C, T, O, S> {
    pub fn new(capture: C, transcriber: T, output: O, sink: S, min_samples: usize) -> Self {
        Self {
            capture,
            transcriber,
            output,
            sink,
            min_samples,
            recording: false,
        }
    }

    pub fn handle(&mut self, trigger: Trigger) -> Result<(), PipelineError> {
        match trigger {
            Trigger::Start => {
                if self.recording {
                    return Ok(());
                }
                self.sink.publish(StatusEvent::Recording);
                if let Err(e) = self.capture.start() {
                    self.sink.publish(StatusEvent::Error(e.clone()));
                    self.sink.publish(StatusEvent::Idle);
                    return Err(PipelineError::Capture(e));
                }
                self.recording = true;
                Ok(())
            }
            Trigger::Stop => {
                if !self.recording {
                    return Ok(());
                }
                let samples = self.capture.stop().map_err(PipelineError::Capture)?;
                self.recording = false;
                self.sink.publish(StatusEvent::Stopped);
                if samples.len() < self.min_samples {
                    self.sink.publish(StatusEvent::Idle);
                    return Ok(());
                }
                self.sink.publish(StatusEvent::Transcribing);
                let text = match self.transcriber.transcribe(&samples) {
                    Ok(t) => t,
                    Err(e) => {
                        self.sink.publish(StatusEvent::Error(e.clone()));
                        self.sink.publish(StatusEvent::Idle);
                        return Err(PipelineError::Transcribe(e));
                    }
                };
                self.sink.publish(StatusEvent::Idle);
                if !text.is_empty() {
                    self.output
                        .deliver(&text)
                        .map_err(PipelineError::Output)?;
                }
                Ok(())
            }
        }
    }

    pub fn run(mut self, rx: Receiver<Trigger>) {
        while let Ok(t) = rx.recv() {
            if let Err(e) = self.handle(t) {
                eprintln!("[hush] pipeline: {e:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct VecCapture {
        inner: Arc<Mutex<VecCaptureState>>,
    }

    #[derive(Default)]
    struct VecCaptureState {
        start_count: usize,
        stop_count: usize,
        samples: Vec<f32>,
        start_err: Option<String>,
        stop_err: Option<String>,
    }

    impl VecCapture {
        fn with_samples(samples: Vec<f32>) -> Self {
            let s = Self::default();
            s.inner.lock().unwrap().samples = samples;
            s
        }
        fn set_start_err(&self, msg: &str) {
            self.inner.lock().unwrap().start_err = Some(msg.to_string());
        }
        fn clear_start_err(&self) {
            self.inner.lock().unwrap().start_err = None;
        }
        fn start_count(&self) -> usize {
            self.inner.lock().unwrap().start_count
        }
        fn stop_count(&self) -> usize {
            self.inner.lock().unwrap().stop_count
        }
    }

    impl Capture for VecCapture {
        fn start(&mut self) -> Result<(), String> {
            let mut g = self.inner.lock().unwrap();
            g.start_count += 1;
            if let Some(e) = g.start_err.clone() {
                return Err(e);
            }
            Ok(())
        }
        fn stop(&mut self) -> Result<Vec<f32>, String> {
            let mut g = self.inner.lock().unwrap();
            g.stop_count += 1;
            if let Some(e) = g.stop_err.clone() {
                return Err(e);
            }
            Ok(g.samples.clone())
        }
    }

    #[derive(Clone)]
    struct CannedTranscriber {
        result: Arc<Result<String, String>>,
    }

    impl CannedTranscriber {
        fn new(text: &str) -> Self {
            Self {
                result: Arc::new(Ok(text.to_string())),
            }
        }
        fn fail(err: &str) -> Self {
            Self {
                result: Arc::new(Err(err.to_string())),
            }
        }
    }

    impl Transcriber for CannedTranscriber {
        fn transcribe(&self, _: &[f32]) -> Result<String, String> {
            (*self.result).clone()
        }
    }

    #[derive(Clone, Default)]
    struct RecordingOutput {
        texts: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingOutput {
        fn texts(&self) -> Vec<String> {
            self.texts.lock().unwrap().clone()
        }
    }

    impl Output for RecordingOutput {
        fn deliver(&self, text: &str) -> Result<(), String> {
            self.texts.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct VecStatusSink {
        events: Arc<Mutex<Vec<StatusEvent>>>,
    }

    impl VecStatusSink {
        fn events(&self) -> Vec<StatusEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl StatusSink for VecStatusSink {
        fn publish(&self, ev: StatusEvent) {
            self.events.lock().unwrap().push(ev);
        }
    }

    fn make_pipeline() -> (
        Pipeline<VecCapture, CannedTranscriber, RecordingOutput, VecStatusSink>,
        VecCapture,
        RecordingOutput,
        VecStatusSink,
    ) {
        let cap = VecCapture::with_samples(vec![0.0; 32_000]);
        let trans = CannedTranscriber::new("hello");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let p = Pipeline::new(cap.clone(), trans, out.clone(), sink.clone(), 4_800);
        (p, cap, out, sink)
    }

    #[test]
    fn start_emits_recording() {
        let (mut p, _cap, _out, sink) = make_pipeline();
        p.handle(Trigger::Start).unwrap();
        assert_eq!(sink.events(), vec![StatusEvent::Recording]);
    }

    #[test]
    fn second_start_is_idempotent() {
        let (mut p, cap, _out, sink) = make_pipeline();
        p.handle(Trigger::Start).unwrap();
        p.handle(Trigger::Start).unwrap();
        assert_eq!(cap.start_count(), 1);
        assert_eq!(sink.events(), vec![StatusEvent::Recording]);
    }

    #[test]
    fn happy_path_delivers_transcript() {
        let (mut p, _cap, out, sink) = make_pipeline();
        p.handle(Trigger::Start).unwrap();
        p.handle(Trigger::Stop).unwrap();
        assert_eq!(
            sink.events(),
            vec![
                StatusEvent::Recording,
                StatusEvent::Stopped,
                StatusEvent::Transcribing,
                StatusEvent::Idle,
            ]
        );
        assert_eq!(out.texts(), vec!["hello".to_string()]);
    }

    #[test]
    fn empty_transcript_does_not_deliver() {
        let cap = VecCapture::with_samples(vec![0.0; 32_000]);
        let trans = CannedTranscriber::new("");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let mut p = Pipeline::new(cap, trans, out.clone(), sink.clone(), 4_800);
        p.handle(Trigger::Start).unwrap();
        p.handle(Trigger::Stop).unwrap();
        assert_eq!(
            sink.events(),
            vec![
                StatusEvent::Recording,
                StatusEvent::Stopped,
                StatusEvent::Transcribing,
                StatusEvent::Idle,
            ]
        );
        assert!(out.texts().is_empty());
    }

    #[test]
    fn stop_without_start_is_noop() {
        let cap = VecCapture::with_samples(vec![0.0; 32_000]);
        let trans = CannedTranscriber::new("hello");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let mut p = Pipeline::new(cap.clone(), trans, out.clone(), sink.clone(), 4_800);
        p.handle(Trigger::Stop).unwrap();
        assert!(sink.events().is_empty());
        assert_eq!(cap.start_count(), 0);
        assert_eq!(cap.stop_count(), 0);
        assert!(out.texts().is_empty());
    }

    #[test]
    fn transcribe_error_emits_error_then_idle_and_skips_deliver() {
        let cap = VecCapture::with_samples(vec![0.0; 32_000]);
        let trans = CannedTranscriber::fail("model crashed");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let mut p = Pipeline::new(cap, trans, out.clone(), sink.clone(), 4_800);
        p.handle(Trigger::Start).unwrap();
        let _ = p.handle(Trigger::Stop);
        assert_eq!(
            sink.events(),
            vec![
                StatusEvent::Recording,
                StatusEvent::Stopped,
                StatusEvent::Transcribing,
                StatusEvent::Error("model crashed".to_string()),
                StatusEvent::Idle,
            ]
        );
        assert!(out.texts().is_empty());
    }

    #[test]
    fn capture_start_error_emits_error_then_idle_and_recovers() {
        let cap = VecCapture::with_samples(vec![0.0; 32_000]);
        cap.set_start_err("device busy");
        let trans = CannedTranscriber::new("hello");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let mut p = Pipeline::new(cap.clone(), trans, out.clone(), sink.clone(), 4_800);

        let _ = p.handle(Trigger::Start);
        assert_eq!(
            sink.events(),
            vec![
                StatusEvent::Recording,
                StatusEvent::Error("device busy".to_string()),
                StatusEvent::Idle,
            ]
        );
        assert!(out.texts().is_empty());

        cap.clear_start_err();
        p.handle(Trigger::Start).unwrap();
        assert_eq!(sink.events().last(), Some(&StatusEvent::Recording));
        assert_eq!(cap.start_count(), 2);
    }

    #[test]
    fn short_clip_drops_without_transcribing() {
        let cap = VecCapture::with_samples(vec![0.0; 100]);
        let trans = CannedTranscriber::new("should not be delivered");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let mut p = Pipeline::new(cap, trans, out.clone(), sink.clone(), 4_800);
        p.handle(Trigger::Start).unwrap();
        p.handle(Trigger::Stop).unwrap();
        assert_eq!(
            sink.events(),
            vec![
                StatusEvent::Recording,
                StatusEvent::Stopped,
                StatusEvent::Idle,
            ]
        );
        assert!(out.texts().is_empty());
    }
}
