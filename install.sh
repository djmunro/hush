#!/bin/bash
set -euo pipefail

REPO_URL="https://github.com/djmunro/hush.git"
INSTALL_DIR="$HOME/.local/share/hush"

if [[ "$(uname)" != "Darwin" ]]; then
  echo "hush is macOS-only." >&2
  exit 1
fi

# If this script lives next to Cargo.toml, use that checkout. Otherwise clone.
SCRIPT_DIR=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]:-/dev/null}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/Cargo.toml" ]]; then
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

if ! command -v cmake >/dev/null; then
  if command -v brew >/dev/null; then
    echo "→ installing cmake via Homebrew (needed to build whisper.cpp)"
    brew install --quiet cmake
  else
    cat >&2 <<EOF
cmake is missing. Install Homebrew first:

  /bin/bash -c "\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

then re-run the hush install command.
EOF
    exit 1
  fi
fi

if ! command -v cargo >/dev/null; then
  if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
  else
    echo "→ installing Rust toolchain via rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
fi

echo "→ building hush (first build downloads + compiles whisper.cpp — ~3–5 min)"
cargo build --release --quiet

BIN="$SRC/target/release/hush"
if [[ ! -x "$BIN" ]]; then
  echo "build did not produce $BIN" >&2
  exit 1
fi

mkdir -p "$HOME/.local/bin"
ln -sf "$BIN" "$HOME/.local/bin/hush"

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
        <string>$BIN</string>
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
# launchd needs a moment to release the label after bootout
for i in 1 2 3 4 5; do
  if launchctl bootstrap "gui/$UID_NUM" "$PLIST" 2>/dev/null; then
    break
  fi
  sleep 1
done

cat <<EOF

✓ hush installed at $SRC

ONE LAST STEP — grant two macOS permissions.

System Settings is opening two panes. In each:
  1. Click the lock to unlock (if needed)
  2. Click the + button
  3. Press Cmd+Shift+G, paste this path, hit Enter:

       $BIN

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
