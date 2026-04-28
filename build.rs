// Capture the short git hash at build time so the binary can show
// "Hush 0.2.0 (abc1234)" — version comes from CARGO_PKG_VERSION,
// hash distinguishes builds between releases.
//
// Falls back to "unknown" if git isn't on PATH or the build is
// outside a working tree (e.g. a release tarball install).

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=HUSH_GIT_HASH={hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
}
