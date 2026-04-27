# hush

Local push-to-talk dictation for macOS. Hold **fn**, talk, release — Whisper transcribes and pastes at your cursor. No cloud, no always-on mic.

## Why

WisprFlow and other dictation tools listen continuously and ship audio off-device. hush does neither: the mic is closed at rest, only opens while you're holding fn, and transcription runs locally on CPU via [faster-whisper](https://github.com/SYSTRAN/faster-whisper).

## Install

```bash
git clone git@github.com:djmunro/hush.git
cd hush
./install.sh
```

The installer creates a Python venv, downloads dependencies, registers a `launchd` agent so hush auto-starts at login, and prints the paths for two macOS permissions you'll need to grant manually.

## Permissions

macOS requires two permissions, both in **System Settings → Privacy & Security**. The installer prints the binary path to add — drag it in or click `+`.

| Permission | Why |
|---|---|
| **Accessibility** | Sends Cmd+V to paste transcribed text |
| **Input Monitoring** | Detects fn key press/release |

After granting, restart the agent:
```bash
launchctl kickstart -k gui/$(id -u)/com.djmunro.hush
```

If macOS' built-in fn behavior gets in the way (emoji picker, dictation), set **System Settings → Keyboard → Press 🌐/fn key to:** to "Do Nothing".

## Usage

- **Press and hold fn** → Tink sound, mic opens
- **Talk**
- **Release fn** → Pop sound, transcription runs, text pastes at cursor

## Customization

| Knob | How |
|---|---|
| Whisper model | `WHISPER_MODEL=base.en hush` (`tiny.en`, `base.en`, `small.en`, `medium.en`) |
| Hotkey | Edit `FN_FLAG` in `hush.py` (uses Quartz `kCGEventFlagMask*` constants) |

`small.en` is the default — best accuracy/speed tradeoff for English on Apple Silicon CPU. `base.en` is ~2× faster with slightly worse accuracy.

## Manage

```bash
hush                                                  # run manually (foreground)
tail -f ~/Library/Logs/hush.log                       # logs
launchctl bootout gui/$(id -u)/com.djmunro.hush       # stop agent
launchctl bootstrap gui/$(id -u) \
  ~/Library/LaunchAgents/com.djmunro.hush.plist       # start agent
./uninstall.sh                                        # remove agent + symlinks
```

## Troubleshooting

**Nothing happens when I press fn.** Check `tail -f ~/Library/Logs/hush.log`. If you see "event tap unavailable" or "Input Monitoring not granted", the binary isn't approved. Re-add it in Input Monitoring and run `launchctl kickstart -k gui/$(id -u)/com.djmunro.hush`.

**Paste fails with "not allowed to send keystrokes".** Accessibility permission missing. Add the same binary to the Accessibility list.

**First word or two gets cut off.** CoreAudio takes ~150–300 ms to open the mic on press. Pause briefly between Tink and speaking.

**Transcription too slow.** Try `WHISPER_MODEL=base.en` or `tiny.en`. Default is `small.en`.

## Stack

- [faster-whisper](https://github.com/SYSTRAN/faster-whisper) — CTranslate2 port of Whisper, int8 quantized
- [sounddevice](https://python-sounddevice.readthedocs.io) — PortAudio bindings for mic input
- Quartz `CGEventTap` — fn key detection (via [pyobjc](https://pyobjc.readthedocs.io))

## License

MIT.
