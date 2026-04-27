#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"
HERE="$(pwd)"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "hush is macOS-only." >&2
  exit 1
fi

if ! command -v python3 >/dev/null; then
  echo "python3 not found. Install Python 3.11+ first (e.g. 'brew install python@3.13')." >&2
  exit 1
fi

if [[ ! -d .venv ]]; then
  echo "→ creating venv"
  python3 -m venv .venv
fi
echo "→ installing dependencies"
./.venv/bin/pip install --quiet --upgrade pip
./.venv/bin/pip install --quiet -r requirements.txt

chmod +x hush

mkdir -p "$HOME/.local/bin"
ln -sf "$HERE/hush" "$HOME/.local/bin/hush"
echo "→ symlinked $HOME/.local/bin/hush"

PLIST="$HOME/Library/LaunchAgents/com.djmunro.hush.plist"
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.djmunro.hush</string>
    <key>ProgramArguments</key>
    <array>
        <string>$HERE/hush</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>$HOME/Library/Logs/hush.log</string>
    <key>StandardErrorPath</key>
    <string>$HOME/Library/Logs/hush.log</string>
</dict>
</plist>
EOF
echo "→ wrote $PLIST"

UID_NUM=$(id -u)
launchctl bootout "gui/$UID_NUM/com.djmunro.hush" 2>/dev/null || true
launchctl bootstrap "gui/$UID_NUM" "$PLIST"
echo "→ launchd agent loaded"

PY_REAL=$(./.venv/bin/python -c "import os, sys; print(os.path.realpath(sys.executable))")

cat <<EOF

✓ installed.

GRANT TWO MACOS PERMISSIONS in System Settings → Privacy & Security:

  1. Accessibility       — for pasting text via Cmd+V
  2. Input Monitoring    — for detecting fn key holds

Add this binary to BOTH lists (drag the file in or click + and paste the path):

  $PY_REAL

Then either restart your Mac or run:

  launchctl kickstart -k "gui/$UID_NUM/com.djmunro.hush"

Usage:
  • Hold fn, talk, release — text is pasted at your cursor.
  • Logs:    tail -f ~/Library/Logs/hush.log
  • Stop:    launchctl bootout gui/$UID_NUM/com.djmunro.hush
  • Start:   launchctl bootstrap gui/$UID_NUM "$PLIST"
  • Manual:  hush
EOF
