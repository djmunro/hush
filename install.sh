#!/bin/bash
set -euo pipefail

REPO_URL="https://github.com/djmunro/hush.git"
INSTALL_DIR="$HOME/.local/share/hush"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "hush is macOS-only." >&2
  exit 1
fi

# If this script lives next to hush.py, use that checkout. Otherwise clone.
SCRIPT_DIR=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]:-/dev/null}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/hush.py" ]]; then
  SRC="$SCRIPT_DIR"
  echo "→ using local checkout at $SRC"
else
  if ! command -v git >/dev/null; then
    cat >&2 <<EOF
git is missing. macOS will now prompt to install the Command Line Tools.
After it finishes (a few minutes), re-run:

  curl -fsSL https://raw.githubusercontent.com/djmunro/hush/main/install.sh | bash
EOF
    xcode-select --install 2>/dev/null || true
    exit 1
  fi
  if [[ -d "$INSTALL_DIR/.git" ]]; then
    echo "→ updating existing checkout at $INSTALL_DIR"
    git -C "$INSTALL_DIR" pull --quiet --ff-only
  else
    echo "→ cloning hush to $INSTALL_DIR"
    mkdir -p "$(dirname "$INSTALL_DIR")"
    git clone --quiet "$REPO_URL" "$INSTALL_DIR"
  fi
  SRC="$INSTALL_DIR"
fi

cd "$SRC"

if ! command -v python3 >/dev/null; then
  if command -v brew >/dev/null; then
    echo "→ installing Python via Homebrew"
    brew install --quiet python@3.13
  else
    cat >&2 <<EOF
Python 3 is missing. Install Homebrew first:

  /bin/bash -c "\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

then re-run the hush install command.
EOF
    exit 1
  fi
fi

if [[ ! -d .venv ]]; then
  echo "→ creating venv"
  python3 -m venv .venv
fi
echo "→ installing dependencies (this can take a minute)"
./.venv/bin/pip install --quiet --upgrade pip
./.venv/bin/pip install --quiet -r requirements.txt

chmod +x hush

mkdir -p "$HOME/.local/bin"
ln -sf "$SRC/hush" "$HOME/.local/bin/hush"

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
        <string>$SRC/hush</string>
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

UID_NUM=$(id -u)
launchctl bootout "gui/$UID_NUM/com.djmunro.hush" 2>/dev/null || true
launchctl bootstrap "gui/$UID_NUM" "$PLIST"

PY_REAL=$(./.venv/bin/python -c "import os, sys; print(os.path.realpath(sys.executable))")

cat <<EOF

✓ hush installed at $SRC

ONE LAST STEP — grant two macOS permissions.

System Settings is opening two panes. In each:
  1. Click the lock to unlock (if needed)
  2. Click the + button
  3. Press Cmd+Shift+G, paste this path, hit Enter:

       $PY_REAL

  4. Click "Open" and toggle the entry ON

Do this for BOTH panes:
  • Accessibility       (so hush can paste text)
  • Input Monitoring    (so hush can detect the fn key)

EOF

open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
sleep 1
open "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"

cat <<EOF

After granting both, restart hush:

    launchctl kickstart -k gui/$UID_NUM/com.djmunro.hush

Then hold fn anywhere on macOS, talk, and release. Text appears at your cursor.

Manual run:  hush
Logs:        tail -f ~/Library/Logs/hush.log
Uninstall:   $SRC/uninstall.sh
EOF
