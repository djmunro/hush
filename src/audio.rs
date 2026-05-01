//! One-shot model bootstrap. The capture/transcribe/output pipeline lives
//! in `crate::dictation`.

use std::path::{Path, PathBuf};
use std::process::Command;

const PARAKEET_MODEL_DIR: &str = "parakeet-tdt-0.6b-v3";
const PARAKEET_URL_PREFIX: &str =
    "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/";
const PARAKEET_FILES: &[&str] = &[
    "encoder-model.onnx",
    "encoder-model.onnx.data",
    "decoder_joint-model.onnx",
    "vocab.txt",
];

const PARAKEET_EOU_MODEL_DIR: &str = "parakeet-eou-120m";
const PARAKEET_EOU_URL_PREFIX: &str =
    "https://huggingface.co/altunenes/parakeet-rs/resolve/main/realtime_eou_120m-v1-onnx/";
const PARAKEET_EOU_FILES: &[&str] = &["encoder.onnx", "decoder_joint.onnx", "tokenizer.json"];

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/models")
}

pub fn ensure_parakeet_model_path() -> PathBuf {
    ensure_parakeet_model()
}

pub fn ensure_parakeet_eou_model_path() -> PathBuf {
    let dir = cache_dir().join(PARAKEET_EOU_MODEL_DIR);
    std::fs::create_dir_all(&dir).expect("create parakeet-eou model dir");
    for &file in PARAKEET_EOU_FILES {
        let path = dir.join(file);
        let url = format!("{PARAKEET_EOU_URL_PREFIX}{file}");
        download_if_missing(&path, &url, file);
    }
    dir
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
