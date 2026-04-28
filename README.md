# hush

Local push-to-talk dictation for macOS. Hold **fn**, talk, release — Whisper transcribes and pastes at your cursor. No cloud, no always-on mic.

Lives in your menubar. Floating pill near the bottom-center of the screen shows live audio levels while you speak and a transcribing animation while Whisper runs.

## Install

### Drag-and-drop (recommended)

Download the latest `Hush-x.y.z.dmg` from the [releases page](https://github.com/djmunro/hush/releases), open it, drag **Hush.app** to **Applications**, then double-click it. The Settings window opens automatically the first time and walks you through the three permissions.

Because the bundle is ad-hoc signed (no Apple Developer ID), you may need to right-click → **Open** the first time so Gatekeeper lets it through.

### Build from source

```bash
git clone https://github.com/djmunro/hush.git
cd hush
bash install.sh        # builds Hush.app, drops it in /Applications, opens it
```

Requires Xcode Command Line Tools (`cmake`, `swift`) and the Rust toolchain (auto-installs via `rustup` if missing). First build is ~3–5 minutes while `whisper.cpp` compiles.

## Permissions

The Settings window has a card for each. The first one prompts in-app; the other two open the relevant System Settings pane.

| Permission | How granted |
|---|---|
| **Microphone** | One-click in-app prompt |
| **Input Monitoring** | Toggle Hush ON in System Settings (so we can detect the fn key globally) |
| **Accessibility** | Toggle Hush ON in System Settings (so we can send Cmd+V to paste) |

Permission status updates within ~1.5s of granting — no need to relaunch.

If macOS' fn behavior gets in the way, set **System Settings → Keyboard → Press 🌐/fn key to:** "Do Nothing".

## Usage

Hold **fn** → Tink, mic opens, the floating pill appears with live audio bars. Talk. Release **fn** → Pop, the pill switches to a transcribing animation, then disappears as the text pastes at your cursor.

`WHISPER_MODEL=base.en open /Applications/Hush.app` to swap models (`tiny.en` / `base.en` / `small.en` / `medium.en`). Default `small.en`. Models cache in `~/.cache/hush/models`.

Auto-start at login: there's a checkbox in the Settings window. (Or use System Settings → General → Login Items → +.)

## Troubleshooting

- **App doesn't launch / Gatekeeper warning.** Right-click Hush.app → Open. Or: `xattr -d com.apple.quarantine /Applications/Hush.app`.
- **Permission stays Denied after I toggle it on in System Settings.** You probably have a stale TCC entry from an earlier install. See `docs/macos-permissions.md` for `tccutil reset` or run `bash uninstall.sh && bash install.sh`.
- **`python3.14` or `hush` (with a generic green icon) appears in System Settings.** Both are leftover from older versions of hush. Click the entry, hit `−`. The new bundle re-registers cleanly under its own bundle ID.
- **First word cut off.** CoreAudio takes ~150–300 ms to open the mic. Pause briefly between Tink and speaking.
- **Paste goes to the wrong app.** Hush sends Cmd+V to whatever app has focus when you release fn. Make sure the target field is focused before you release.

## Dev workflow

After any code change:

```bash
bash scripts/install-dev.sh
```

Builds the bundle, kills the running instance, `tccutil reset`s hush's TCC entries (because ad-hoc signing produces a fresh cdhash on every build that TCC treats as a new identity), copies to `/Applications/Hush.app`, and opens it. Re-grant the perms once after each install.

For a release build:

```bash
bash scripts/package.sh    # → dist/Hush-x.y.z.{dmg,zip}
```

See [`docs/architecture.md`](docs/architecture.md) for module map, threading model, AppKit patterns. See [`docs/macos-permissions.md`](docs/macos-permissions.md) for everything we learned about TCC, code signing, hardened runtime, and the dev TCC reset workflow.

## Stack

[whisper.cpp](https://github.com/ggerganov/whisper.cpp) via [whisper-rs](https://github.com/tazz4843/whisper-rs) (Metal on Apple Silicon) · [cpal](https://github.com/RustAudio/cpal) for mic · AppKit via [objc2](https://github.com/madsmtm/objc2) · `CGEventTap` for fn key · `CGEventPost` for paste.

## Uninstall

```bash
bash uninstall.sh
```

Quits the running app, removes `/Applications/Hush.app`, removes any auto-start LaunchAgent, and resets hush's TCC entries. Leaves the model cache (`~/.cache/hush`) in place.

MIT.
