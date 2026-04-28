//! Audio capture (cpal) + whisper inference, on a worker thread.
//!
//! The main thread runs NSApp; it must not block. The event tap pushes
//! `Msg::Start` / `Msg::Stop` onto a channel, and this worker owns the
//! cpal stream and whisper context.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::keyboard;
use crate::overlay::{OverlayMode, OverlayState};

const SAMPLE_RATE: u32 = 16_000;
const MIN_SAMPLES: usize = (SAMPLE_RATE as f32 * 0.3) as usize;

const START_SOUND: &str = "/System/Library/Sounds/Tink.aiff";
const STOP_SOUND: &str = "/System/Library/Sounds/Pop.aiff";

const DEFAULT_MODEL: &str = "small.en";
const MODEL_URL_PREFIX: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-";

#[derive(Debug)]
pub enum Msg {
    Start,
    Stop,
}

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/models")
}

pub fn ensure_model() -> PathBuf {
    let model = std::env::var("WHISPER_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let filename = format!("ggml-{model}.bin");
    let url = format!("{MODEL_URL_PREFIX}{model}.bin");

    let dir = cache_dir();
    std::fs::create_dir_all(&dir).expect("create model dir");
    let path = dir.join(&filename);
    if path.exists() {
        return path;
    }
    eprintln!("[hush] downloading {filename}…");
    let tmp = path.with_extension("bin.part");
    let status = Command::new("curl")
        .args(["-fL", "--retry", "3", "-o"])
        .arg(&tmp)
        .arg(&url)
        .status()
        .expect("run curl");
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("[hush] model download failed");
        std::process::exit(1);
    }
    std::fs::rename(&tmp, &path).expect("move model into place");
    path
}

fn play(sound: &'static str) {
    let _ = Command::new("afplay")
        .arg(sound)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

struct Recorder {
    buf: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
    src_rate: u32,
    overlay: Arc<Mutex<OverlayState>>,
}

impl Recorder {
    fn new(overlay: Arc<Mutex<OverlayState>>) -> Self {
        Self {
            buf: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            src_rate: SAMPLE_RATE,
            overlay,
        }
    }

    fn start(&mut self) -> Result<(), String> {
        if self.stream.is_some() {
            return Ok(());
        }
        self.buf.lock().unwrap().clear();

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no input device".to_string())?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?;
        let sample_format = supported.sample_format();
        let channels = supported.channels();
        self.src_rate = supported.sample_rate();
        let config: cpal::StreamConfig = supported.into();

        let buf = Arc::clone(&self.buf);
        let err_cb = |err| eprintln!("[hush] stream error: {err}");

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let buf = Arc::clone(&buf);
                let overlay = Arc::clone(&self.overlay);
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _| {
                        append_mono(&buf, &overlay, data, channels, |s| s);
                    },
                    err_cb,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let buf = Arc::clone(&buf);
                let overlay = Arc::clone(&self.overlay);
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        append_mono(&buf, &overlay, data, channels, |s| s as f32 / 32768.0);
                    },
                    err_cb,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let buf = Arc::clone(&buf);
                let overlay = Arc::clone(&self.overlay);
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        append_mono(&buf, &overlay, data, channels, |s| {
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

        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) -> Vec<f32> {
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
            drop(stream);
        }
        let mono = std::mem::take(&mut *self.buf.lock().unwrap());
        resample_to_16k(&mono, self.src_rate)
    }
}

fn append_mono<S: Copy>(
    buf: &Arc<Mutex<Vec<f32>>>,
    overlay: &Arc<Mutex<OverlayState>>,
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

    // Compute RMS of the just-appended chunk and push it to the
    // overlay so the bars track live mic input. This runs on the cpal
    // audio thread; the lock is uncontended in practice (UI reads at
    // 30Hz, audio writes at ~50Hz).
    let new_slice = &guard[start_len..];
    if !new_slice.is_empty() {
        let sum_sq: f32 = new_slice.iter().map(|x| x * x).sum();
        let rms = (sum_sq / new_slice.len() as f32).sqrt();
        // Map to a useful display range — typical speech RMS is 0.02-0.2,
        // so amplify to fill the bar-height domain.
        let display = (rms * 6.0).min(1.0);
        drop(guard);
        OverlayState::push_level(overlay, display);
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

pub fn run_worker(model_path: &Path, rx: Receiver<Msg>, overlay: Arc<Mutex<OverlayState>>) {
    eprintln!("[hush] loading model…");
    let ctx = WhisperContext::new_with_params(
        model_path.to_str().expect("utf-8 model path"),
        WhisperContextParameters::default(),
    )
    .expect("load whisper model");

    let mut rec = Recorder::new(Arc::clone(&overlay));
    eprintln!("[hush] ready. hold fn to dictate.");

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Start => {
                play(START_SOUND);
                OverlayState::set_mode(&overlay, OverlayMode::Recording);
                if let Err(e) = rec.start() {
                    eprintln!("[hush] failed to start recording: {e}");
                    OverlayState::set_mode(&overlay, OverlayMode::Hidden);
                }
            }
            Msg::Stop => {
                let audio = rec.stop();
                play(STOP_SOUND);
                if audio.len() < MIN_SAMPLES {
                    eprintln!("[hush] too short, skipping");
                    OverlayState::set_mode(&overlay, OverlayMode::Hidden);
                    continue;
                }
                OverlayState::set_mode(&overlay, OverlayMode::Transcribing);
                let t0 = Instant::now();
                let result = transcribe(&ctx, &audio);
                OverlayState::set_mode(&overlay, OverlayMode::Hidden);
                let elapsed = t0.elapsed().as_secs_f32();
                match result {
                    Ok(text) if text.is_empty() => {
                        eprintln!("[hush] no speech detected ({:.1}s)", elapsed);
                    }
                    Ok(text) => {
                        eprintln!("[hush] ({:.1}s) {}", elapsed, text);
                        if let Err(e) = keyboard::paste(&text) {
                            eprintln!("[hush] paste failed: {e}");
                        }
                    }
                    Err(e) => eprintln!("[hush] {e}"),
                }
            }
        }
    }
}

fn transcribe(ctx: &WhisperContext, audio: &[f32]) -> Result<String, String> {
    let mut state = ctx.create_state().map_err(|e| format!("state error: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_no_context(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    state
        .full(params, audio)
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
