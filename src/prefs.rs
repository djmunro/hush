use std::path::PathBuf;

fn prefs_path() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset")).join(".cache/hush/backend")
}

/// Returns the active backend name. Env var HUSH_BACKEND takes priority.
pub fn get_backend() -> String {
    if let Ok(v) = std::env::var("HUSH_BACKEND") {
        return v;
    }
    std::fs::read_to_string(prefs_path())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "whisper".to_string())
}

pub fn set_backend(backend: &str) {
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, backend);
}
