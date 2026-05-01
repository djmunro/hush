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
    /// Non-blocking: returns the next 2560-sample chunk (at 16kHz) if available, None otherwise.
    fn drain_chunk(&mut self) -> Option<Vec<f32>> {
        None
    }
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

pub trait StreamTranscriber: Send {
    fn start_session(&mut self) -> Result<(), String>;
    /// Feed a 2560-sample chunk; returns any text decoded so far (may be empty).
    fn feed(&mut self, chunk: &[f32]) -> Result<String, String>;
    /// Called on Stop with the final partial chunk; returns any remaining text.
    fn flush(&mut self, remaining: &[f32]) -> Result<String, String>;
    fn sample_count(&self) -> usize;
}

pub struct Pipeline<C, T, ST, O, S> {
    capture: C,
    transcriber: T,
    stream_transcriber: Option<ST>,
    output: O,
    sink: S,
    min_samples: usize,
    recording: bool,
    streaming: bool,
}

impl<C: Capture, T: Transcriber, ST: StreamTranscriber, O: Output, S: StatusSink>
    Pipeline<C, T, ST, O, S>
{
    pub fn new(
        capture: C,
        transcriber: T,
        stream_transcriber: Option<ST>,
        output: O,
        sink: S,
        min_samples: usize,
        streaming: bool,
    ) -> Self {
        Self {
            capture,
            transcriber,
            stream_transcriber,
            output,
            sink,
            min_samples,
            recording: false,
            streaming,
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
                if self.streaming {
                    if let Some(st) = self.stream_transcriber.as_mut() {
                        if let Err(e) = st.start_session() {
                            self.sink.publish(StatusEvent::Error(e.clone()));
                            self.sink.publish(StatusEvent::Idle);
                            return Err(PipelineError::Transcribe(e));
                        }
                    }
                }
                self.recording = true;
                Ok(())
            }
            Trigger::Stop => {
                if !self.recording {
                    return Ok(());
                }
                self.recording = false;
                self.sink.publish(StatusEvent::Stopped);

                if self.streaming {
                    let remaining = self.capture.stop().map_err(PipelineError::Capture)?;
                    let st = self.stream_transcriber.as_mut().ok_or_else(|| {
                        PipelineError::Transcribe("no streaming transcriber".to_string())
                    })?;
                    if st.sample_count() + remaining.len() < self.min_samples {
                        self.sink.publish(StatusEvent::Idle);
                        return Ok(());
                    }
                    self.sink.publish(StatusEvent::Transcribing);
                    let text = match st.flush(&remaining) {
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
                } else {
                    let samples = self.capture.stop().map_err(PipelineError::Capture)?;
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
                }
                Ok(())
            }
        }
    }

    fn drain_streaming_chunks(&mut self) {
        let Some(st) = self.stream_transcriber.as_mut() else {
            return;
        };
        while let Some(chunk) = self.capture.drain_chunk() {
            match st.feed(&chunk) {
                Ok(text) if !text.is_empty() => {
                    if let Err(e) = self.output.deliver(&text) {
                        eprintln!("[hush] streaming output: {e}");
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("[hush] streaming feed: {e}"),
            }
        }
    }

    pub fn run(mut self, rx: Receiver<Trigger>) {
        loop {
            let timeout = if self.recording && self.streaming {
                std::time::Duration::from_millis(160)
            } else {
                std::time::Duration::from_secs(3600)
            };

            match rx.recv_timeout(timeout) {
                Ok(t) => {
                    if let Err(e) = self.handle(t) {
                        eprintln!("[hush] pipeline: {e:?}");
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    self.drain_streaming_chunks();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
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

    struct NoopStreamTranscriber;

    impl StreamTranscriber for NoopStreamTranscriber {
        fn start_session(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn feed(&mut self, _chunk: &[f32]) -> Result<String, String> {
            Ok(String::new())
        }
        fn flush(&mut self, _remaining: &[f32]) -> Result<String, String> {
            Ok(String::new())
        }
        fn sample_count(&self) -> usize {
            0
        }
    }

    fn make_pipeline() -> (
        Pipeline<VecCapture, CannedTranscriber, NoopStreamTranscriber, RecordingOutput, VecStatusSink>,
        VecCapture,
        RecordingOutput,
        VecStatusSink,
    ) {
        let cap = VecCapture::with_samples(vec![0.0; 32_000]);
        let trans = CannedTranscriber::new("hello");
        let out = RecordingOutput::default();
        let sink = VecStatusSink::default();
        let p = Pipeline::new(cap.clone(), trans, None::<NoopStreamTranscriber>, out.clone(), sink.clone(), 4_800, false);
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
        let mut p = Pipeline::new(cap, trans, None::<NoopStreamTranscriber>, out.clone(), sink.clone(), 4_800, false);
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
        let mut p = Pipeline::new(cap.clone(), trans, None::<NoopStreamTranscriber>, out.clone(), sink.clone(), 4_800, false);
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
        let mut p = Pipeline::new(cap, trans, None::<NoopStreamTranscriber>, out.clone(), sink.clone(), 4_800, false);
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
        let mut p = Pipeline::new(cap.clone(), trans, None::<NoopStreamTranscriber>, out.clone(), sink.clone(), 4_800, false);

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
        let mut p = Pipeline::new(cap, trans, None::<NoopStreamTranscriber>, out.clone(), sink.clone(), 4_800, false);
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
