# hush

Local push-to-talk dictation for macOS. Hold your shortcut (default **fn**), talk, release — Whisper transcribes and pastes at your cursor. No cloud, no always-on mic.

Lives in your menubar. Floating pill near the bottom-center of the screen shows live audio levels while you speak and a transcribing animation while Whisper runs.

Two macOS permissions: **Microphone** and **Accessibility**. No Input Monitoring (we use `NSEvent.addGlobalMonitor` for the shortcut, which Accessibility already covers).

## Install

### Homebrew (recommended)

```bash
brew tap djmunro/hush https://github.com/djmunro/hush.git
brew install --cask hush
```

The first command points Brew at this repo as a tap (we don't have a separate `homebrew-hush` repo — the cask lives at `Casks/hush.rb` here). You only run it once per machine. After that, `brew install`, `brew upgrade`, and `brew uninstall` all work normally. Open Hush from Launchpad or `/Applications` — the Settings window auto-opens the first time and walks you through the two permissions.

### Direct download

Grab the latest `Hush-x.y.z.dmg` from the [releases page](https://github.com/djmunro/hush/releases), open it, and drag **Hush.app** to **Applications**.

Because the bundle is ad-hoc signed (no Apple Developer ID), Gatekeeper may complain on first launch. Either right-click → **Open**, or:

```bash
xattr -d com.apple.quarantine /Applications/Hush.app
```

(Homebrew strips the quarantine bit for you, which is why `brew install` skips the warning.)

### Build from source

```bash
git clone https://github.com/djmunro/hush.git
cd hush
bash scripts/install-dev.sh
```

Requires Xcode Command Line Tools (`cmake`, `swift`) and the Rust toolchain. First build is ~3–5 minutes while `whisper.cpp` compiles.

## Update

```bash
brew upgrade --cask hush
```

## Uninstall

```bash
brew uninstall --cask --zap hush
```

`--zap` also removes the model cache (`~/.cache/hush`, can be 100MB–2GB), preferences, and saved app state. Without `--zap`, only `Hush.app` and the autostart LaunchAgent are removed.

macOS TCC permissions (Microphone, Accessibility) survive uninstall by design — Apple keeps them keyed to the bundle ID. To wipe them:

```bash
tccutil reset Microphone com.djmunro.hush
tccutil reset Accessibility com.djmunro.hush
```

### Migrating from a source install

If you previously installed via `bash install.sh` (now removed), run this once before `brew install` to clear the legacy LaunchAgent and TCC entries:

```bash
launchctl bootout "gui/$(id -u)/com.djmunro.hush" 2>/dev/null || true
rm -f ~/Library/LaunchAgents/com.djmunro.hush.plist
rm -rf /Applications/Hush.app
for svc in Microphone Accessibility ListenEvent; do
  tccutil reset "$svc" com.djmunro.hush 2>/dev/null || true
done
```

Then `brew install --cask hush` from a clean slate.

## Permissions

Two cards in the Settings window. The first prompts in-app; the second opens System Settings.

| Permission | How granted | Why |
|---|---|---|
| **Microphone** | One-click in-app prompt | Capture audio. |
| **Accessibility** | Toggle Hush ON in System Settings | Detect the push-to-talk shortcut globally (`NSEvent.addGlobalMonitor`) AND send Cmd+V to paste (`CGEventPost`) — one perm covers both. |

Permission status updates within ~1.5s of granting — no need to relaunch.

If macOS' fn behavior gets in the way, set **System Settings → Keyboard → Press 🌐/fn key to:** "Do Nothing".

## Usage

Hold the shortcut (default **fn**) → Tink, mic opens, the floating pill appears with live audio bars. Talk. Release → Pop, the pill switches to a transcribing animation, then disappears as the text pastes at your cursor.

### Customizing the shortcut

Settings → **Push-to-talk** card → **Record…**, then press the keys you want and release. Modifiers (incl. left vs. right side: `L⌘` ≠ `R⌘`) plus one optional non-modifier key. Press **Esc** to cancel.

Stored at `~/.config/hush/config.toml` (`shortcut = "..."`, e.g. `"fn"`, `"left_cmd+space"`, `"left_option+right_option"`). Hand-editable.

### Models

`WHISPER_MODEL=base.en open /Applications/Hush.app` to swap models (`tiny.en` / `base.en` / `small.en` / `medium.en`). Default `small.en`. Models cache in `~/.cache/hush/models`.

Auto-start at login: there's a checkbox in the Settings window. (Or use System Settings → General → Login Items → +.)

## Troubleshooting

- **App doesn't launch / Gatekeeper warning** (direct-download install only). Right-click Hush.app → Open. Or: `xattr -d com.apple.quarantine /Applications/Hush.app`. `brew install` doesn't hit this because Homebrew strips the quarantine bit.
- **Permission stays Denied after I toggle it on in System Settings.** Stale TCC entry from an earlier install. `tccutil reset Microphone com.djmunro.hush && tccutil reset Accessibility com.djmunro.hush`, then re-toggle.
- **`python3.14` or `hush` (with a generic green icon) appears in System Settings.** Both are leftover from older builds. Click the entry, hit `−`. The new bundle re-registers cleanly under its own bundle ID.
- **First word cut off.** CoreAudio takes ~150–300 ms to open the mic. Pause briefly between Tink and speaking.
- **Paste goes to the wrong app.** Hush sends Cmd+V to whatever app has focus when you release the shortcut. Make sure the target field is focused before you release.
- **Shortcut conflicts with another app.** Pick a different one in Settings → Push-to-talk → Record. The default `fn` rarely collides; combos like `R⌥` or `L⌘+Space` are common alternatives. Note macOS reserves a few system-wide chords (Spotlight on `Cmd+Space`, etc.).

## Stack

[whisper.cpp](https://github.com/ggerganov/whisper.cpp) via [whisper-rs](https://github.com/tazz4843/whisper-rs) (Metal on Apple Silicon) · [cpal](https://github.com/RustAudio/cpal) for mic · AppKit via [objc2](https://github.com/madsmtm/objc2) · `NSEvent.addGlobalMonitor` for the fn key · `CGEventPost` for paste.

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

Cutting a release: see [`.claude/skills/cut-release/SKILL.md`](.claude/skills/cut-release/SKILL.md) and [`docs/release.md`](docs/release.md).

[whisper.cpp](https://github.com/ggerganov/whisper.cpp) via [whisper-rs](https://github.com/tazz4843/whisper-rs) (Metal on Apple Silicon) · [cpal](https://github.com/RustAudio/cpal) for mic · AppKit via [objc2](https://github.com/madsmtm/objc2) · `NSEvent.addGlobalMonitor` + `addLocalMonitor` for the shortcut · `CGEventPost` for paste.

## Uninstall

```bash
bash uninstall.sh
```

Quits the running app, removes `/Applications/Hush.app`, removes any auto-start LaunchAgent, and resets hush's TCC entries. Leaves the model cache (`~/.cache/hush`) in place.

See [`docs/architecture.md`](docs/architecture.md) for module map, threading model, AppKit patterns. See [`docs/macos-permissions.md`](docs/macos-permissions.md) for everything we learned about TCC, code signing, hardened runtime, and the dev TCC reset workflow.

MIT.
