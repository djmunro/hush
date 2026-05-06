//! cpal capture isolated to its own thread. `cpal::Stream` is `!Send`, so
//! it lives entirely inside the worker thread; the orchestrator drives
//! capture through mpsc commands. The stream is built lazily on first
//! `Start` and kept alive thereafter — subsequent `Start`/`Stop` call
//! stream.play()/pause() so the CoreAudio unit only runs while recording,
//! which clears the macOS orange microphone indicator between recordings.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::pipeline::Capture;

const SAMPLE_RATE: u32 = 16_000;

type LevelCb = Arc<dyn Fn(f32) + Send + Sync + 'static>;

enum Cmd {
    Start {
        reply: mpsc::Sender<Result<(), String>>,
    },
    Stop {
        reply: mpsc::Sender<Result<Vec<f32>, String>>,
    },
}

pub struct CpalCapture {
    cmd_tx: mpsc::Sender<Cmd>,
}

impl CpalCapture {
    pub fn new<F>(level_cb: F) -> Self
    where
        F: Fn(f32) + Send + Sync + 'static,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let cb: LevelCb = Arc::new(level_cb);
        std::thread::spawn(move || run(cmd_rx, cb));
        Self { cmd_tx }
    }
}

impl Capture for CpalCapture {
    fn start(&mut self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(Cmd::Start { reply: tx })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }

    fn stop(&mut self) -> Result<Vec<f32>, String> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(Cmd::Stop { reply: tx })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
}

struct StreamState {
    stream: cpal::Stream,
    src_rate: u32,
    recording: Arc<AtomicBool>,
    buf: Arc<Mutex<Vec<f32>>>,
}

fn run(rx: mpsc::Receiver<Cmd>, level_cb: LevelCb) {
    let mut state: Option<StreamState> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Start { reply } => {
                if state.is_none() {
                    match start_stream(level_cb.clone()) {
                        Ok(s) => state = Some(s),
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            continue;
                        }
                    }
                }
                let s = state.as_ref().unwrap();
                s.buf.lock().unwrap().clear();
                s.recording.store(true, Ordering::Relaxed);
                if let Err(e) = s.stream.play() {
                    let _ = reply.send(Err(format!("stream play: {e}")));
                    continue;
                }
                let _ = reply.send(Ok(()));
            }
            Cmd::Stop { reply } => {
                let Some(s) = state.as_ref() else {
                    let _ = reply.send(Ok(Vec::new()));
                    continue;
                };
                s.recording.store(false, Ordering::Relaxed);
                let mono = std::mem::take(&mut *s.buf.lock().unwrap());
                if let Err(e) = s.stream.pause() {
                    eprintln!("[hush] stream pause: {e}");
                }
                let _ = reply.send(Ok(resample_to_16k(&mono, s.src_rate)));
            }
        }
    }
}

fn start_stream(level_cb: LevelCb) -> Result<StreamState, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no input device".to_string())?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("default_input_config: {e}"))?;
    let sample_format = supported.sample_format();
    let channels = supported.channels();
    let src_rate = supported.sample_rate();
    let config: cpal::StreamConfig = supported.into();

    let recording = Arc::new(AtomicBool::new(false));
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let err_cb = |err| eprintln!("[hush] stream error: {err}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            let buf = Arc::clone(&buf);
            let cb = Arc::clone(&level_cb);
            let rec = Arc::clone(&recording);
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    if !rec.load(Ordering::Relaxed) {
                        return;
                    }
                    append_mono(&buf, &cb, data, channels, |s| s);
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let buf = Arc::clone(&buf);
            let cb = Arc::clone(&level_cb);
            let rec = Arc::clone(&recording);
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    if !rec.load(Ordering::Relaxed) {
                        return;
                    }
                    append_mono(&buf, &cb, data, channels, |s| s as f32 / 32768.0);
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let buf = Arc::clone(&buf);
            let cb = Arc::clone(&level_cb);
            let rec = Arc::clone(&recording);
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    if !rec.load(Ordering::Relaxed) {
                        return;
                    }
                    append_mono(&buf, &cb, data, channels, |s| {
                        (s as f32 - 32768.0) / 32768.0
                    });
                },
                err_cb,
                None,
            )
        }
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| e.to_string())?;

    Ok(StreamState {
        stream,
        src_rate,
        recording,
        buf,
    })
}

fn append_mono<S: Copy>(
    buf: &Arc<Mutex<Vec<f32>>>,
    level_cb: &LevelCb,
    data: &[S],
    channels: u16,
    to_f32: impl Fn(S) -> f32,
) {
    let n = channels as usize;
    let mut guard = buf.lock().unwrap();
    let start_len = guard.len();
    if n <= 1 {
        guard.extend(data.iter().copied().map(to_f32));
    } else {
        let inv = 1.0 / n as f32;
        for frame in data.chunks_exact(n) {
            let sum: f32 = frame.iter().copied().map(&to_f32).sum();
            guard.push(sum * inv);
        }
    }
    let new_slice = &guard[start_len..];
    if !new_slice.is_empty() {
        let sum_sq: f32 = new_slice.iter().map(|x| x * x).sum();
        let rms = (sum_sq / new_slice.len() as f32).sqrt();
        let display = (rms * 6.0).min(1.0);
        drop(guard);
        level_cb(display);
    }
}

fn resample_to_16k(input: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == SAMPLE_RATE || input.is_empty() {
        return input.to_vec();
    }
    let ratio = src_rate as f64 / SAMPLE_RATE as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    let last = input.len() - 1;
    for i in 0..out_len {
        let src = i as f64 * ratio;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(last);
        let frac = (src - lo as f64) as f32;
        out.push(input[lo] * (1.0 - frac) + input[hi] * frac);
    }
    out
}
