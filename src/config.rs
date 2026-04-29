//! User config at ~/.config/hush/config.toml.
//!
//! Single field today: the push-to-talk shortcut. Stored as a string like
//! `"fn"`, `"left_option+space"`, or `"left_cmd+right_cmd"` so it stays
//! human-editable. Autostart is *not* stored here — it's derived from the
//! presence of ~/Library/LaunchAgents/com.djmunro.hush.plist, which is the
//! actual source of truth for whether launchd will start us.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModKey {
    Fn,
    LeftCommand,
    RightCommand,
    LeftOption,
    RightOption,
    LeftControl,
    RightControl,
    LeftShift,
    RightShift,
    CapsLock,
}

impl ModKey {
    fn as_str(self) -> &'static str {
        match self {
            ModKey::Fn => "fn",
            ModKey::LeftCommand => "left_cmd",
            ModKey::RightCommand => "right_cmd",
            ModKey::LeftOption => "left_option",
            ModKey::RightOption => "right_option",
            ModKey::LeftControl => "left_ctrl",
            ModKey::RightControl => "right_ctrl",
            ModKey::LeftShift => "left_shift",
            ModKey::RightShift => "right_shift",
            ModKey::CapsLock => "capslock",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "fn" => ModKey::Fn,
            "left_cmd" | "lcmd" => ModKey::LeftCommand,
            "right_cmd" | "rcmd" => ModKey::RightCommand,
            "left_option" | "left_opt" | "lopt" | "lalt" => ModKey::LeftOption,
            "right_option" | "right_opt" | "ropt" | "ralt" => ModKey::RightOption,
            "left_ctrl" | "lctrl" => ModKey::LeftControl,
            "right_ctrl" | "rctrl" => ModKey::RightControl,
            "left_shift" | "lshift" => ModKey::LeftShift,
            "right_shift" | "rshift" => ModKey::RightShift,
            "capslock" | "caps" => ModKey::CapsLock,
            _ => return None,
        })
    }

    pub fn pretty(self) -> &'static str {
        match self {
            ModKey::Fn => "fn",
            ModKey::LeftCommand => "L⌘",
            ModKey::RightCommand => "R⌘",
            ModKey::LeftOption => "L⌥",
            ModKey::RightOption => "R⌥",
            ModKey::LeftControl => "L⌃",
            ModKey::RightControl => "R⌃",
            ModKey::LeftShift => "L⇧",
            ModKey::RightShift => "R⇧",
            ModKey::CapsLock => "⇪",
        }
    }
}

/// Shortcut binding. Modifiers are an unordered set; `key` is an optional
/// non-modifier (a virtual keycode). If `key` is None the chord fires on
/// modifiers-only (like the default `fn`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub mods: Vec<ModKey>,
    pub key: Option<u16>,
    pub key_label: Option<String>,
}

impl Shortcut {
    pub fn fn_only() -> Self {
        Self {
            mods: vec![ModKey::Fn],
            key: None,
            key_label: None,
        }
    }

    pub fn pretty(&self) -> String {
        let mut parts: Vec<String> = self.mods.iter().map(|m| m.pretty().to_string()).collect();
        if let Some(label) = &self.key_label {
            parts.push(label.clone());
        }
        if parts.is_empty() {
            "(none)".to_string()
        } else {
            parts.join(" + ")
        }
    }

    fn to_token(&self) -> String {
        let mut parts: Vec<String> = self.mods.iter().map(|m| m.as_str().to_string()).collect();
        if let (Some(code), Some(label)) = (self.key, &self.key_label) {
            parts.push(format!("key:{code}:{label}"));
        } else if let Some(code) = self.key {
            parts.push(format!("key:{code}"));
        }
        parts.join("+")
    }

    fn from_token(s: &str) -> Option<Self> {
        let mut mods = Vec::new();
        let mut key = None;
        let mut key_label = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            if let Some(rest) = part.strip_prefix("key:") {
                let mut it = rest.splitn(2, ':');
                let code: u16 = it.next()?.parse().ok()?;
                key = Some(code);
                key_label = it.next().map(|s| s.to_string());
            } else if let Some(m) = ModKey::parse(part) {
                if !mods.contains(&m) {
                    mods.push(m);
                }
            } else {
                return None;
            }
        }
        if mods.is_empty() && key.is_none() {
            return None;
        }
        Some(Self {
            mods,
            key,
            key_label,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Whisper,
    Parakeet,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Whisper => "whisper",
            BackendKind::Parakeet => "parakeet",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "whisper" => Some(BackendKind::Whisper),
            "parakeet" => Some(BackendKind::Parakeet),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConfigFile {
    shortcut: Option<String>,
    backend: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub shortcut: Shortcut,
    pub backend: BackendKind,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shortcut: Shortcut::fn_only(),
            backend: BackendKind::Parakeet,
        }
    }
}

fn config_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset"))
        .join(".config")
        .join("hush")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load() -> Config {
    let path = config_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return Config::default();
    };
    let Ok(parsed) = toml::from_str::<ConfigFile>(&text) else {
        eprintln!("[hush] config.toml is malformed; using defaults");
        return Config::default();
    };
    let shortcut = parsed
        .shortcut
        .as_deref()
        .and_then(Shortcut::from_token)
        .unwrap_or_else(Shortcut::fn_only);
    let backend = std::env::var("HUSH_BACKEND")
        .ok()
        .as_deref()
        .and_then(BackendKind::parse)
        .or_else(|| parsed.backend.as_deref().and_then(BackendKind::parse))
        .unwrap_or(BackendKind::Parakeet);
    Config { shortcut, backend }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {dir:?}: {e}"))?;
    let body = ConfigFile {
        shortcut: Some(cfg.shortcut.to_token()),
        backend: Some(cfg.backend.as_str().to_string()),
    };
    let text = toml::to_string_pretty(&body).map_err(|e| e.to_string())?;
    fs::write(config_path(), text).map_err(|e| e.to_string())
}
