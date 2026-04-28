#!/bin/bash
# hush installer — builds Hush.app from source and drops it in
# /Applications. After install, the app opens once; the Settings window
# auto-launches and walks through the three permissions.
#
# No LaunchAgent: hush is LSUIElement=true (a menubar accessory). If the
# user wants auto-start at login, the Settings window has a checkbox
# for it.

set -euo pipefail

REPO_URL="https://github.com/djmunro/hush.git"
INSTALL_DIR="$HOME/.local/share/hush"
DEST_APP="/Applications/Hush.app"

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
        cat >&2 <<'EOF'
git is missing. macOS will now prompt to install the Command Line Tools.
After it finishes (a few minutes), re-run the install command.
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
        cat >&2 <<'EOF'
cmake is missing. Install Homebrew first:

  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

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

# Build the .app bundle (cargo build + Info.plist + .icns + ad-hoc sign).
# Hardened runtime is deliberately OFF — see docs/macos-permissions.md.
echo "→ building Hush.app (first build takes ~3–5 min while whisper.cpp compiles)"
bash "$SRC/scripts/build-app.sh"

BUILT_APP="$SRC/target/release/Hush.app"

# Quit any running instance so we don't fight the file copy.
osascript -e 'tell application "Hush" to quit' 2>/dev/null || true
pkill -f "Hush.app/Contents/MacOS/hush" 2>/dev/null || true

# Clean up the legacy LaunchAgent + symlink from the pre-bundle install layout.
LEGACY_PLIST="$HOME/Library/LaunchAgents/com.djmunro.hush.plist"
if [[ -f "$LEGACY_PLIST" ]]; then
    echo "→ removing legacy LaunchAgent (pre-bundle install)"
    launchctl bootout "gui/$(id -u)/com.djmunro.hush" 2>/dev/null || true
    rm -f "$LEGACY_PLIST"
fi
[[ -L "$HOME/.local/bin/hush" ]] && rm -f "$HOME/.local/bin/hush"

# Reset TCC for our bundle ID — clears any sticky Denied state from a
# previous (now-invalidated) cdhash. Harmless on first install.
for svc in Microphone Accessibility ListenEvent; do
    tccutil reset "$svc" com.djmunro.hush >/dev/null 2>&1 || true
done

# Swap the installed bundle.
echo "→ installing to $DEST_APP"
sleep 0.5
rm -rf "$DEST_APP"
cp -R "$BUILT_APP" "$DEST_APP"

cat <<EOF

✓ installed Hush.app at $DEST_APP

What happens now:
  - Hush.app appears in your menubar (top-right).
  - Settings window auto-opens because perms are unset.
  - Click "Allow microphone" → standard system prompt → Allow.
  - Click "Open Input Monitoring…" → toggle Hush ON in System Settings.
  - Click "Open Accessibility…" → toggle Hush ON in System Settings.
  - The Settings window has an "Open at login" checkbox.

Then hold the fn key anywhere on macOS, talk, release. Text appears at your cursor.

Stale entries to clean up MANUALLY in System Settings →
Privacy & Security → Microphone / Input Monitoring / Accessibility:
  - "hush" with the generic green exec icon (old bare-binary install)
  - "python3.14" (old osascript misattribution — fixed in current builds)

Click each and hit the - button. The new Hush.app will register fresh
under its own bundle ID, com.djmunro.hush.

Uninstall:  bash $SRC/uninstall.sh
Dev loop:   bash $SRC/scripts/install-dev.sh   (rebuilds + re-grants)
EOF

open "$DEST_APP"
