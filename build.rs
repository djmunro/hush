use std::process::Command;

fn main() {
    let hash = git_short_hash();
    println!("cargo:rustc-env=HUSH_GIT_HASH={hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");
}

fn git_short_hash() -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success());
    match out {
        Some(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        None => "unknown".to_string(),
    }
}
