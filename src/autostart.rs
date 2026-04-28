//! Open Hush.app at login via a per-user LaunchAgent.
//!
//! Why a LaunchAgent instead of SMAppService?
//!   - Works on macOS 10.13+ (SMAppService is 13.0+).
//!   - No new framework dependency (objc2-service-management).
//!   - Plain plist on disk is trivial to inspect / nuke if it goes wrong.
//!
//! The plist points launchd at `/usr/bin/open <Hush.app>` rather than
//! the binary directly, so the bundle's TCC identity (com.djmunro.hush)
//! is what gets attributed when the app starts.
//!
//! KeepAlive is deliberately FALSE — earlier versions of hush had
//! KeepAlive=true and the app would relaunch every time the user quit
//! it from the menubar. That bug wasted hours of frustration.

use std::path::PathBuf;
use std::process::Command;

const LABEL: &str = "com.djmunro.hush";

fn plist_path() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME unset"))
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn app_bundle_path() -> Option<PathBuf> {
    // Resolve from the running executable: <bundle>/Contents/MacOS/hush.
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    if bundle.extension().is_some_and(|ext| ext == "app") {
        Some(bundle.to_path_buf())
    } else {
        // Fall back to the canonical install path so the dev workflow
        // (running from /Applications/Hush.app) still works even when
        // current_exe resolves through symlinks oddly.
        Some(PathBuf::from("/Applications/Hush.app"))
    }
}

pub fn is_enabled() -> bool {
    plist_path().exists()
}

pub fn enable() -> Result<(), String> {
    let bundle = app_bundle_path().ok_or_else(|| "could not resolve Hush.app path".to_string())?;
    let plist = plist_path();

    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create LaunchAgents dir: {e}"))?;
    }

    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
"#,
        bundle.display()
    );
    std::fs::write(&plist, body).map_err(|e| format!("write plist: {e}"))?;

    // Bootstrap into launchd so it's active for this session, not just
    // on next login. Bootout first in case a stale entry exists.
    let domain = format!("gui/{}", uid());
    let target = format!("{domain}/{LABEL}");
    let _ = Command::new("launchctl")
        .args(["bootout", &target])
        .status();
    Command::new("launchctl")
        .args(["bootstrap", &domain])
        .arg(&plist)
        .status()
        .map_err(|e| format!("launchctl bootstrap: {e}"))?;

    Ok(())
}

pub fn disable() -> Result<(), String> {
    let domain = format!("gui/{}", uid());
    let target = format!("{domain}/{LABEL}");
    let _ = Command::new("launchctl")
        .args(["bootout", &target])
        .status();
    let plist = plist_path();
    if plist.exists() {
        std::fs::remove_file(&plist).map_err(|e| format!("remove plist: {e}"))?;
    }
    Ok(())
}

fn uid() -> u32 {
    // Avoid pulling in libc just for getuid. Shell out to `id -u`.
    let out = Command::new("id")
        .arg("-u")
        .output()
        .expect("id -u");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(501)
}
