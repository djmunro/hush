//! One-shot model bootstrap. The capture/transcribe/output pipeline lives
//! in `crate::dictation`.

use std::path::{Path, PathBuf};
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
    ensure_backend_model(crate::prefs::get_backend() == "parakeet")
}

pub fn ensure_backend_model(use_parakeet: bool) -> Backend {
    if use_parakeet {
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
    download_if_missing(&path, &url, &filename);
    path
}

fn ensure_parakeet_model() -> PathBuf {
    let dir = cache_dir().join(PARAKEET_MODEL_DIR);
    std::fs::create_dir_all(&dir).expect("create parakeet model dir");
    for &file in PARAKEET_FILES {
        let path = dir.join(file);
        let url = format!("{PARAKEET_URL_PREFIX}{file}");
        download_if_missing(&path, &url, file);
    }
    dir
}

fn download_if_missing(path: &Path, url: &str, display_name: &str) {
    if path.exists() {
        return;
    }
    eprintln!("[hush] downloading {display_name}…");
    let tmp = path.with_extension("part");
    let status = Command::new("curl")
        .args(["-fL", "--retry", "3", "-o"])
        .arg(&tmp)
        .arg(url)
        .status()
        .expect("run curl");
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("[hush] download failed: {display_name}");
        std::process::exit(1);
    }
    std::fs::rename(&tmp, path).expect("move model file into place");
}
