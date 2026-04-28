#!/bin/bash
set -euo pipefail

# Stop and remove the legacy LaunchAgent (older installs only).
launchctl bootout "gui/$(id -u)/com.djmunro.hush" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/com.djmunro.hush.plist"
rm -f "$HOME/.local/bin/hush"

# Quit any running Hush.app, then remove it.
osascript -e 'tell application "Hush" to quit' 2>/dev/null || true
pkill -f 'Hush.app/Contents/MacOS/hush' 2>/dev/null || true
rm -rf "$HOME/Applications/Hush.app"

echo "uninstalled. (model cache at ~/.cache/hush left in place)"
echo
echo "If \"hush\" or \"python3.14\" still appear in System Settings →"
echo "Privacy & Security → Input Monitoring / Accessibility, click each"
echo "and hit the - button to fully remove."
