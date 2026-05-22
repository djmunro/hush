//! One-shot model bootstrap. The capture/transcribe/output pipeline lives
//! in `crate::dictation`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::config::ParakeetModel;

const EXPECTED_FILES: &[&str] = &[
    "encoder-model.onnx",
    "encoder-model.onnx.data",
    "decoder_joint-model.onnx",
    "decoder_joint-model.onnx.data",
    "vocab.txt",
];

pub fn cache_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/models")
}

#[derive(Deserialize, Debug)]
struct ModelInfo {
    siblings: Vec<RepoFile>,
}

#[derive(Deserialize, Debug)]
struct RepoFile {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<LfsInfo>,
}

#[derive(Deserialize, Debug)]
struct LfsInfo {
    size: u64,
}

fn fetch_model_info(model_id: &str) -> Result<ModelInfo, String> {
    let url = format!("https://huggingface.co/api/models/{model_id}?blobs=true");
    let output = Command::new("curl")
        .args(["-fLs", &url])
        .output()
        .map_err(|e| format!("run curl: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "curl failed querying model info (status {:?}): {}",
            output.status, stderr
        ));
    }
    let info: ModelInfo = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("parse model info JSON: {e}"))?;
    Ok(info)
}

/// Returns the model directory for Parakeet, downloading missing files first.
pub fn ensure_model_for(model: ParakeetModel) -> Result<PathBuf, String> {
    set_download_status(DownloadStatus::Idle);
    let dir = cache_dir().join(model.cache_dir_name());
    std::fs::create_dir_all(&dir).map_err(|e| {
        let err = format!("create parakeet model dir: {e}");
        set_download_status(DownloadStatus::Error(err.clone()));
        err
    })?;

    // Query model info to get exact sizes and find which files are actually in the repo
    set_download_status(DownloadStatus::QueryingModelInfo);
    let model_info = fetch_model_info(model.model_id()).map_err(|e| {
        let err = format!("fetch model info failed: {e}");
        set_download_status(DownloadStatus::Error(err.clone()));
        err
    })?;

    // Filter down to the files we actually want to download that exist in the repository
    let mut files_to_check = Vec::new();
    for &file in EXPECTED_FILES {
        if model_info.siblings.iter().any(|s| s.rfilename == file) {
            files_to_check.push(file);
        }
    }

    // Find which of these are missing on disk
    let mut missing_with_sizes = Vec::new();
    let mut total_missing_bytes = 0;
    for file in files_to_check {
        let path = dir.join(file);
        if !path.exists() {
            let size = model_info
                .siblings
                .iter()
                .find(|f| f.rfilename == file)
                .map(|f| f.lfs.as_ref().map(|l| l.size).or(f.size).unwrap_or(0))
                .unwrap_or(0);
            missing_with_sizes.push((file, size));
            total_missing_bytes += size;
        }
    }

    if missing_with_sizes.is_empty() {
        set_download_status(DownloadStatus::Idle);
        return Ok(dir);
    }

    let prefix = model.download_url_prefix();
    let mut completed_bytes = 0;

    for (file, size) in missing_with_sizes {
        let path = dir.join(file);
        let url = format!("{prefix}{file}");
        if let Err(e) = download_if_missing(&path, &url, file, completed_bytes, total_missing_bytes) {
            set_download_status(DownloadStatus::Error(e.clone()));
            return Err(e);
        }
        completed_bytes += size;
    }

    set_download_status(DownloadStatus::Idle);
    Ok(dir)
}

pub fn is_model_cached(model: ParakeetModel) -> bool {
    let dir = cache_dir().join(model.cache_dir_name());
    let files: &[&str] = match model {
        ParakeetModel::V06b => &[
            "encoder-model.onnx",
            "encoder-model.onnx.data",
            "decoder_joint-model.onnx",
            "vocab.txt",
        ],
        ParakeetModel::V11b => &[
            "encoder-model.onnx",
            "encoder-model.onnx.data",
            "decoder_joint-model.onnx",
            "decoder_joint-model.onnx.data",
            "vocab.txt",
        ],
    };
    dir.exists() && files.iter().all(|file| dir.join(file).exists())
}

fn download_if_missing(
    path: &Path,
    url: &str,
    display_name: &str,
    completed_bytes: u64,
    total_bytes: u64,
) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    set_download_status(DownloadStatus::Downloading {
        file: display_name.to_string(),
        downloaded_bytes: completed_bytes,
        total_bytes,
    });
    eprintln!("[hush] downloading {display_name}…");
    let tmp = path.with_extension("part");

    let mut child = Command::new("curl")
        .args(["-fL", "--retry", "3", "-o"])
        .arg(&tmp)
        .arg(url)
        .spawn()
        .map_err(|e| format!("failed to spawn curl: {e}"))?;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let _ = std::fs::remove_file(&tmp);
                    eprintln!("[hush] download failed: {display_name} with status {status}");
                    return Err(format!("download failed: {display_name}"));
                }
                break;
            }
            Ok(None) => {
                let current_part_size = std::fs::metadata(&tmp)
                    .map(|m| m.len())
                    .unwrap_or(0);
                set_download_status(DownloadStatus::Downloading {
                    file: display_name.to_string(),
                    downloaded_bytes: completed_bytes + current_part_size,
                    total_bytes,
                });
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&tmp);
                eprintln!("[hush] waiting for curl child failed: {e}");
                return Err(format!("error waiting for curl: {e}"));
            }
        }
    }

    std::fs::rename(&tmp, path).map_err(|e| {
        eprintln!("[hush] failed to rename tmp file: {e}");
        format!("move model file: {e}")
    })?;
    eprintln!("[hush] successfully downloaded {display_name}");
    Ok(())
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

    fn model_id(self) -> &'static str {
        match self {
            ParakeetModel::V06b => "istupakov/parakeet-tdt-0.6b-v3-onnx",
            ParakeetModel::V11b => "dtgagnon/parakeet-tdt-1.1b-onnx",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DownloadStatus {
    Idle,
    QueryingModelInfo,
    Downloading {
        file: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Error(String),
}

static DOWNLOAD_STATUS: OnceLock<Mutex<DownloadStatus>> = OnceLock::new();

pub fn get_download_status() -> DownloadStatus {
    DOWNLOAD_STATUS
        .get_or_init(|| Mutex::new(DownloadStatus::Idle))
        .lock()
        .unwrap()
        .clone()
}

pub fn set_download_status(status: DownloadStatus) {
    *DOWNLOAD_STATUS
        .get_or_init(|| Mutex::new(DownloadStatus::Idle))
        .lock()
        .unwrap() = status;
}
