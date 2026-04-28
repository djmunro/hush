//! One-shot model bootstrap. The capture/transcribe/output pipeline lives
//! in `crate::dictation`.

use std::path::PathBuf;
use std::process::Command;

const DEFAULT_MODEL: &str = "large-v3-turbo";
const MODEL_URL_PREFIX: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-";

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
