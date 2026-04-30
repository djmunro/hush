use std::path::PathBuf;

const DEFAULT_POST_PROCESS_MODEL: &str = "qwen2.5:0.5b";

fn prefs_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush")
}

#[allow(dead_code)]
fn backend_path() -> PathBuf {
    prefs_dir().join("backend")
}

fn post_process_enabled_path() -> PathBuf {
    prefs_dir().join("post-process-enabled")
}

fn post_process_model_path() -> PathBuf {
    prefs_dir().join("post-process-model")
}

/// Returns the active backend name. Env var HUSH_BACKEND takes priority.
#[allow(dead_code)]
pub fn get_backend() -> String {
    if let Ok(v) = std::env::var("HUSH_BACKEND") {
        return v;
    }
    std::fs::read_to_string(backend_path())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "whisper".to_string())
}

#[allow(dead_code)]
pub fn set_backend(backend: &str) {
    let path = backend_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, backend);
}

pub fn get_post_process_enabled() -> bool {
    if cfg!(test) {
        return false;
    }
    std::fs::read_to_string(post_process_enabled_path())
        .map(|s| s.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn set_post_process_enabled(enabled: bool) {
    let path = post_process_enabled_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, if enabled { "true" } else { "false" });
}

pub fn get_post_process_model() -> String {
    match std::fs::read_to_string(post_process_model_path()) {
        Ok(model) => {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                DEFAULT_POST_PROCESS_MODEL.to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => DEFAULT_POST_PROCESS_MODEL.to_string(),
    }
}

pub fn set_post_process_model(model: &str) {
    let path = post_process_model_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, model);
}
