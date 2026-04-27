#!/bin/bash
set -euo pipefail
UID_NUM=$(id -u)
launchctl bootout "gui/$UID_NUM/com.djmunro.hush" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/com.djmunro.hush.plist"
rm -f "$HOME/.local/bin/hush"
echo "uninstalled. (model cache at ~/.cache/hush left in place)"
