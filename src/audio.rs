//! One-shot model bootstrap. The capture/transcribe/output pipeline lives
//! in `crate::dictation`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ParakeetModel;

const PARAKEET_FILES: &[&str] = &[
    "encoder-model.onnx",
    "encoder-model.onnx.data",
    "decoder_joint-model.onnx",
    "vocab.txt",
];

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/models")
}

/// Returns the model directory for Parakeet, downloading missing files first.
pub fn ensure_model_for(model: ParakeetModel) -> PathBuf {
    let dir = cache_dir().join(model.cache_dir_name());
    std::fs::create_dir_all(&dir).expect("create parakeet model dir");
    let prefix = model.download_url_prefix();
    for &file in PARAKEET_FILES {
        let path = dir.join(file);
        let url = format!("{prefix}{file}");
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

impl ParakeetModel {
    fn cache_dir_name(self) -> &'static str {
        match self {
            ParakeetModel::V06b => "parakeet-tdt-0.6b-v3",
            ParakeetModel::V11b => "parakeet-tdt-1.1b",
        }
    }

    fn download_url_prefix(self) -> &'static str {
        match self {
            ParakeetModel::V06b => {
                "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/"
            }
            ParakeetModel::V11b => {
                "https://huggingface.co/dtgagnon/parakeet-tdt-1.1b-onnx/resolve/main/"
            }
        }
    }
}
