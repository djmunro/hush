# hush — project memory

Local push-to-talk dictation for macOS. Hold fn → talk → release → text appears
at your cursor. Rust + AppKit (objc2) + whisper.cpp (Metal) + cpal.

## Workflow rules

- **Always run `cargo check` AND `cargo clippy --release` before declaring a
  task done.** Zero warnings tolerated. clippy `--fix --allow-dirty` is fine.
- **After any code change, run `bash scripts/install-dev.sh`** — it builds the
  bundle, kills the running app, `tccutil reset`s the TCC entries (because
  ad-hoc signing produces a fresh cdhash every build), copies to
  `/Applications/Hush.app`, and re-opens. This is the canonical dev install.
- **Never run from `target/release/Hush.app` directly.** Always use
  `/Applications/Hush.app` so TCC keys off a stable path.
- For distribution artifacts: `bash scripts/package.sh` → `.dmg` + `.zip` in
  `dist/`.
- `cargo bundle` is *not* in the toolchain. We use `scripts/build-app.sh`
  (manual bash) — shorter than configuring a third-party tool.
- **Cutting a release**: see `.claude/skills/cut-release/SKILL.md`. TL;DR:
  bump `Cargo.toml` version, `git tag vX.Y.Z`, push tag, CI does the rest
  (build, package, GitHub Release, Homebrew cask bump). Long-form pipeline
  in `docs/release.md`.

## Platform invariants — read before changing

- **Never shell out to `osascript` for keystrokes.** macOS attributes
  Accessibility prompts to the *responsible process* in the launch tree, so
  `osascript` from a Terminal session whose ancestor is `python3.14` triggers
  a "python3.14 wants to send keystrokes" prompt. Use native `CGEventPost`
  (see `src/keyboard.rs`).
- **Never sign with `--options runtime` (hardened runtime) in ad-hoc dev
  builds.** Hardened runtime gates mic/Accessibility/etc. on explicit
  `Entitlements.plist` entries; without them, macOS silently denies the
  request *and* doesn't fire a TCC prompt. See `scripts/build-app.sh`.
- **Use `libc::_exit(0)` to terminate, not `NSApplication::terminate`.**
  `terminate` calls `libc::exit` which runs C++ atexit destructors, which
  triggers a `ggml-metal` residency-set assertion crash. See `src/ui.rs`
  `quit:` selector.
- **Use the right TCC API per service:**
  - Microphone → `AVCaptureDevice::requestAccessForMediaType(AVMediaTypeAudio)`
    (in-app popup, no System Settings detour).
  - Accessibility → `AXIsProcessTrustedWithOptions(prompt:true)` —
    canonical "register + prompt", reliably adds the binary to the System
    Settings list.
- **Never use `CGEventTap` for global key listening.** It requires the
  separate Input Monitoring TCC perm (`kTCCServiceListenEvent`), and
  `CGEventTapCreate` against an unauthorized cdhash silently re-fires
  the "Keystroke Receiving" prompt — a prompt loop we spent hours
  debugging. Use `NSEvent::addGlobalMonitorForEventsMatchingMask` with
  `NSEventMask::FlagsChanged` instead — same fn-key detection, gated
  only on Accessibility (which we already need for `CGEventPost`).
  See `src/main.rs::install_fn_monitor` and
  `docs/macos-permissions.md` for the full rationale.

## Code style

- Default to no comments. Only when *why* is non-obvious (a workaround for a
  known macOS quirk, a non-obvious safety constraint, a perf trade-off).
- No emojis in code. Plain mark glyphs in user-visible text only when load-bearing.
- Module layout: one concern per file. `keyboard.rs`, `audio.rs`,
  `overlay.rs`, `ui.rs`, `icon.rs`, `perms.rs`, `main.rs`.
- objc2 0.6 conventions: `define_class!`, `MainThreadMarker`,
  `MainThreadOnly` for AppKit types, `AllocAnyThread` for plain `NSObject`.
- Threading: NSApp main loop only; audio + whisper on a worker thread; the
  cpal stream is `!Send` so it must live on whatever thread starts it.
- Shared overlay state via `Arc<Mutex<OverlayState>>` — audio thread mutates,
  UI thread reads under a 30Hz NSTimer.

## When something doesn't work

- Permission stuck Denied / no prompt fires → see `docs/macos-permissions.md`.
- Build / sign / TCC identity questions → `docs/macos-permissions.md`.
- AppKit / objc2 / overlay layout questions → `docs/architecture.md`.
- App appears in System Settings as `python3.14` or with a generic exec icon
  → both are TCC stale-entry symptoms; see `docs/macos-permissions.md`.

## Don't

- Don't add new LaunchAgents. There is exactly one, owned by
  `src/autostart.rs`, that backs the "Open Hush at login" checkbox in
  Settings. Anything else (KeepAlive watchdogs, IPC daemons, etc.) is
  out of scope — the bundle is `LSUIElement=true` and lives in the
  menubar; users start it manually if they don't want autostart.
  Long-term, autostart should migrate to `SMAppService` (macOS 13+).
- Don't reintroduce `install.sh` / `uninstall.sh`. Distribution lives in
  the Homebrew cask at `Casks/hush.rb` in this repo (this repo IS the
  tap, via `brew tap djmunro/hush https://github.com/djmunro/hush.git`).
  Users run `brew install --cask hush` and `brew uninstall --cask --zap hush`.
  The cask's `uninstall` block handles the LaunchAgent + process kill +
  plist removal. See `docs/release.md`.
- Don't reintroduce a `~/.local/bin/hush` symlink — the bare binary at a
  separate path creates a separate TCC identity, which is the bug we spent
  most of this project debugging.
- Don't reintroduce the Input Monitoring TCC perm. Anything we'd do with
  `CGEventTap` we can do with `NSEvent.addGlobalMonitor` under Accessibility.
- **Do** `git add Cargo.lock` — bin crates check it in (Cargo's official
  guidance for end products: reproducible builds across machines).
- Don't `git add dist/` (gitignored).
- Don't leave stale processes around. `install-dev.sh` `pkill -9`s every
  `hush`-named process and waits up to 3s for them to exit before
  swapping the bundle — old processes hold the OLD cdhash registered
  with TCC and can re-fire prompts behind your back.
