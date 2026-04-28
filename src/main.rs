//! hush — local push-to-talk dictation. Hold fn, talk, release to paste.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const SAMPLE_RATE: u32 = 16_000;
const MIN_SAMPLES: usize = (SAMPLE_RATE as f32 * 0.3) as usize;
const FN_FLAG_BITS: u64 = 0x00800000; // kCGEventFlagMaskSecondaryFn

const START_SOUND: &str = "/System/Library/Sounds/Tink.aiff";
const STOP_SOUND: &str = "/System/Library/Sounds/Pop.aiff";

const DEFAULT_MODEL: &str = "small.en";
const MODEL_URL_PREFIX: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-";

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/models")
}

fn ensure_model() -> PathBuf {
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn paste(text: &str) -> Result<(), String> {
    let prev = Command::new("pbpaste")
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();

    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pbcopy: {e}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "pbcopy stdin".to_string())?
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    child.wait().map_err(|e| e.to_string())?;

    let result = Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to keystroke \"v\" using command down",
        ])
        .output()
        .map_err(|e| format!("osascript: {e}"))?;

    let restore = || {
        if let Ok(mut c) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            if let Some(stdin) = c.stdin.as_mut() {
                let _ = stdin.write_all(&prev);
            }
            let _ = c.wait();
        }
    };

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        std::thread::sleep(Duration::from_millis(250));
        restore();
        if stderr.contains("1002") || stderr.contains("not allowed to send keystrokes") {
            return Err(
                "Accessibility permission missing. Grant in System Settings → Privacy & \
                 Security → Accessibility."
                    .into(),
            );
        }
        return Err(stderr.trim().to_string());
    }

    std::thread::sleep(Duration::from_millis(250));
    restore();
    Ok(())
}

struct Recorder {
    buf: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
    src_rate: u32,
}

impl Recorder {
    fn new() -> Self {
        Self {
            buf: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            src_rate: SAMPLE_RATE,
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
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _| append_mono(&buf, data, channels, |s| s),
                err_cb,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    append_mono(&buf, data, channels, |s| s as f32 / 32768.0)
                },
                err_cb,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    append_mono(&buf, data, channels, |s| {
                        (s as f32 - 32768.0) / 32768.0
                    })
                },
                err_cb,
                None,
            ),
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
    data: &[S],
    channels: u16,
    to_f32: impl Fn(S) -> f32,
) {
    let n = channels as usize;
    let mut guard = buf.lock().unwrap();
    if n <= 1 {
        guard.extend(data.iter().copied().map(to_f32));
    } else {
        let inv = 1.0 / n as f32;
        for frame in data.chunks_exact(n) {
            let sum: f32 = frame.iter().copied().map(&to_f32).sum();
            guard.push(sum * inv);
        }
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

struct Dictator {
    ctx: WhisperContext,
    rec: Recorder,
    fn_down: bool,
}

impl Dictator {
    fn new(model_path: &Path) -> Self {
        eprintln!("[hush] loading model…");
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().expect("utf-8 model path"),
            WhisperContextParameters::default(),
        )
        .expect("load whisper model");
        Self {
            ctx,
            rec: Recorder::new(),
            fn_down: false,
        }
    }

    fn handle_flag(&mut self, fn_pressed: bool) {
        if fn_pressed && !self.fn_down {
            self.fn_down = true;
            play(START_SOUND);
            if let Err(e) = self.rec.start() {
                eprintln!("[hush] failed to start recording: {e}");
            }
            return;
        }
        if !fn_pressed && self.fn_down {
            self.fn_down = false;
            let audio = self.rec.stop();
            play(STOP_SOUND);

            if audio.len() < MIN_SAMPLES {
                eprintln!("[hush] too short, skipping");
                return;
            }
            let t0 = Instant::now();
            let mut state = match self.ctx.create_state() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[hush] state error: {e}");
                    return;
                }
            };
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some("en"));
            params.set_no_context(true);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            if let Err(e) = state.full(params, &audio) {
                eprintln!("[hush] transcribe error: {e}");
                return;
            }
            let n = state.full_n_segments();
            let mut text = String::new();
            for i in 0..n {
                if let Some(seg) = state.get_segment(i) {
                    if let Ok(s) = seg.to_str_lossy() {
                        text.push_str(&s);
                    }
                }
            }
            let text = text.trim().to_string();
            let elapsed = t0.elapsed().as_secs_f32();
            if text.is_empty() {
                eprintln!("[hush] no speech detected ({:.1}s)", elapsed);
                return;
            }
            eprintln!("[hush] ({:.1}s) {}", elapsed, text);
            if let Err(e) = paste(&text) {
                eprintln!("[hush] paste failed: {e}");
            }
        }
    }
}

fn main() {
    unsafe {
        if !CGPreflightListenEventAccess() {
            CGRequestListenEventAccess();
            if !CGPreflightListenEventAccess() {
                eprintln!(
                    "[hush] Input Monitoring not granted. Add this binary in System Settings → \
                     Privacy & Security → Input Monitoring, then relaunch."
                );
                std::process::exit(1);
            }
        }
    }

    let model_path = ensure_model();
    let dictator = RefCell::new(Dictator::new(&model_path));
    eprintln!("[hush] ready. hold fn to dictate.");

    let installed = CGEventTap::with_enabled(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![CGEventType::FlagsChanged],
        |_proxy, _ty, event| {
            let flags = event.get_flags();
            let pressed = (flags.bits() & FN_FLAG_BITS) != 0;
            dictator.borrow_mut().handle_flag(pressed);
            CallbackResult::Keep
        },
        || CFRunLoop::run_current(),
    );

    if installed.is_err() {
        eprintln!(
            "[hush] event tap unavailable. Ensure this binary has Input Monitoring \
             permission and relaunch."
        );
        std::process::exit(1);
    }
}
