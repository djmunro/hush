#!/bin/bash
# hush installer — builds the .app bundle and drops it in ~/Applications.
# After install, open Hush.app once; it lives in your menubar with a
# Settings window guiding the macOS permission grants.
#
# No LaunchAgent: macOS Login Items handles "open at login" via the
# Settings app, which is the modern way for menubar accessory apps.

set -euo pipefail

REPO_URL="https://github.com/djmunro/hush.git"
INSTALL_DIR="$HOME/.local/share/hush"
APPS_DIR="$HOME/Applications"

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

# Toolchain dependencies
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

# Build .app bundle (cargo build + Info.plist + .icns + ad-hoc sign)
echo "→ building Hush.app (first build downloads + compiles whisper.cpp — ~3–5 min)"
bash "$SRC/scripts/build-app.sh"

BUILT_APP="$SRC/target/release/Hush.app"
DEST_APP="$APPS_DIR/Hush.app"

mkdir -p "$APPS_DIR"
rm -rf "$DEST_APP"
cp -R "$BUILT_APP" "$DEST_APP"

# Clean up legacy LaunchAgent + symlink from older installs.
LEGACY_PLIST="$HOME/Library/LaunchAgents/com.djmunro.hush.plist"
if [[ -f "$LEGACY_PLIST" ]]; then
    echo "→ removing legacy LaunchAgent (the new bundle uses Login Items instead)"
    launchctl bootout "gui/$(id -u)/com.djmunro.hush" 2>/dev/null || true
    rm -f "$LEGACY_PLIST"
fi
if [[ -L "$HOME/.local/bin/hush" ]]; then
    rm -f "$HOME/.local/bin/hush"
fi

cat <<EOF

✓ installed Hush.app at $DEST_APP

Next:
  1. Open Hush.app — it lives in your menubar (top-right).
  2. The Settings window auto-opens if any permission is missing:
     • Microphone        — single in-app prompt (one click).
     • Input Monitoring  — opens System Settings, toggle Hush ON.
     • Accessibility     — opens System Settings, toggle Hush ON.
  3. (Optional) For "open at login": System Settings → General → Login
     Items → + → Hush.app.

Then hold the fn key anywhere on macOS, talk, release. Text appears at your cursor.

If you previously installed the bare-binary version of hush, you may
have stale entries named "python3.14" or "hush" in System Settings →
Privacy & Security → Input Monitoring / Accessibility. Click each one
and hit the - button to remove. The new bundle re-registers under its
own bundle ID (${BUNDLE_ID:-com.djmunro.hush}), so future grants stick.

Manual run:  open "$DEST_APP"
Uninstall:   $SRC/uninstall.sh
EOF

open "$DEST_APP"
