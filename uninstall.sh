#!/bin/bash
# Removes Hush.app, any auto-start LaunchAgent, the legacy bare-binary
# symlink, and resets the TCC entries for our bundle ID. Leaves the
# whisper model cache (~/.cache/hush) in place — re-installing later
# skips the model download.

set -euo pipefail

# Quit any running instance.
osascript -e 'tell application "Hush" to quit' 2>/dev/null || true
pkill -f 'Hush.app/Contents/MacOS/hush' 2>/dev/null || true
pkill -x hush 2>/dev/null || true

# Remove the auto-start LaunchAgent (whether installed via the new
# in-app checkbox or the legacy install.sh).
launchctl bootout "gui/$(id -u)/com.djmunro.hush" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/com.djmunro.hush.plist"

# Legacy bare-binary symlink.
rm -f "$HOME/.local/bin/hush"

# The bundle.
rm -rf "/Applications/Hush.app"
rm -rf "$HOME/Applications/Hush.app"

# Reset TCC for our bundle ID so a future reinstall starts clean.
# ListenEvent included to clean up any legacy Input Monitoring grants
# from older builds (current builds don't request that perm).
for svc in Microphone Accessibility ListenEvent; do
    tccutil reset "$svc" com.djmunro.hush >/dev/null 2>&1 || true
done

cat <<EOF
uninstalled.

Left alone:
  - ~/.cache/hush (whisper model cache — saves the redownload on reinstall)
  - ~/.local/share/hush (source clone, if you used the source install)

Manual cleanup if you want a complete reset:
  - System Settings → Privacy & Security → Microphone / Accessibility /
    Input Monitoring — click any "hush" or "python3.14" entries and
    hit - to remove them. (Current builds only use Microphone +
    Accessibility; an Input Monitoring entry from an older build is
    safe to delete.)
EOF
