# hush

Local push-to-talk dictation for macOS. Hold **fn**, talk, release — Whisper transcribes and pastes at your cursor. No cloud, no always-on mic.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/djmunro/hush/main/install.sh | bash
```

Installs `cmake` + the Rust toolchain if missing, builds the binary (~3–5 min the first time while whisper.cpp compiles), registers a `launchd` agent, and opens the two permission panes you need.

## Permissions

Both in **System Settings → Privacy & Security**. Add `~/.local/share/hush/target/release/hush` (or wherever your checkout is) to:

| Pane | Why |
|---|---|
| Input Monitoring | Detect fn key |
| Accessibility    | Send Cmd+V to paste |

After granting:
```bash
launchctl kickstart -k gui/$(id -u)/com.djmunro.hush
```

If macOS' fn behavior gets in the way, set **Settings → Keyboard → Press 🌐/fn key to:** "Do Nothing".

## Usage

Hold **fn** → Tink, mic opens. Talk. Release **fn** → Pop, transcribe, paste.

`WHISPER_MODEL=base.en hush` to swap models (`tiny.en` / `base.en` / `small.en` / `medium.en`). Default `small.en`. Models cache in `~/.cache/hush/models`.

## Troubleshooting

- **Nothing happens on fn.** `tail -f ~/Library/Logs/hush.log`. If you see "Input Monitoring not granted" or "event tap unavailable", the binary isn't approved — re-add it and `launchctl kickstart -k gui/$(id -u)/com.djmunro.hush`.
- **Permissions broke after rebuild.** macOS ties grants to the binary's content hash. Remove and re-add the binary in both Privacy panes after each `cargo build --release`.
- **Paste fails with "not allowed to send keystrokes".** Add the binary to Accessibility.
- **First word cut off.** CoreAudio takes ~150–300 ms to open the mic. Pause briefly between Tink and speaking.

## Stack

[whisper.cpp](https://github.com/ggerganov/whisper.cpp) via [whisper-rs](https://github.com/tazz4843/whisper-rs) (Metal on Apple Silicon) · [cpal](https://github.com/RustAudio/cpal) for mic · `CGEventTap` for fn key.

MIT.
