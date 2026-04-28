//! One-shot model bootstrap. The capture/transcribe/output pipeline lives
//! in `crate::dictation`.

use std::path::PathBuf;
use std::process::Command;

const DEFAULT_WHISPER_MODEL: &str = "large-v3-turbo";
const WHISPER_URL_PREFIX: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-";

const PARAKEET_MODEL_DIR: &str = "parakeet-tdt-0.6b-v3";
const PARAKEET_URL_PREFIX: &str =
    "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/";
const PARAKEET_FILES: &[&str] = &[
    "encoder-model.onnx",
    "encoder-model.onnx.data",
    "decoder_joint-model.onnx",
    "vocab.txt",
];

pub enum Backend {
    Whisper(PathBuf),
    Parakeet(PathBuf),
}

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/models")
}

/// Returns the active backend with its model path, downloading files as needed.
/// Preference is read from prefs (HUSH_BACKEND env var overrides).
pub fn ensure_model() -> Backend {
    if crate::prefs::get_backend() == "parakeet" {
        Backend::Parakeet(ensure_parakeet_model())
    } else {
        Backend::Whisper(ensure_whisper_model())
    }
}

fn ensure_whisper_model() -> PathBuf {
    let model =
        std::env::var("WHISPER_MODEL").unwrap_or_else(|_| DEFAULT_WHISPER_MODEL.to_string());
    let filename = format!("ggml-{model}.bin");
    let url = format!("{WHISPER_URL_PREFIX}{model}.bin");

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

fn ensure_parakeet_model() -> PathBuf {
    let dir = cache_dir().join(PARAKEET_MODEL_DIR);
    std::fs::create_dir_all(&dir).expect("create parakeet model dir");
    for &file in PARAKEET_FILES {
        let path = dir.join(file);
        if path.exists() {
            continue;
        }
        let url = format!("{PARAKEET_URL_PREFIX}{file}");
        eprintln!("[hush] downloading {file}…");
        let tmp = dir.join(format!("{file}.part"));
        let status = Command::new("curl")
            .args(["-fL", "--retry", "3", "-o"])
            .arg(&tmp)
            .arg(&url)
            .status()
            .expect("run curl");
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("[hush] parakeet download failed: {file}");
            std::process::exit(1);
        }
        std::fs::rename(&tmp, &path).expect("move parakeet file into place");
    }
    dir
}
