#!/bin/bash
# Produces release artifacts in dist/:
#   - Hush-<version>.dmg  (drag-and-drop installer with /Applications shortcut)
#   - Hush-<version>.zip  (zipped app bundle, fallback for piped installs)
#
# Both wrap target/release/Hush.app, built fresh by build-app.sh.
# hdiutil + ditto are built into macOS — no Homebrew needed.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
APP_NAME="Hush"
DIST_DIR="$ROOT/dist"
APP_PATH="$ROOT/target/release/${APP_NAME}.app"

# Build the bundle first.
bash "$ROOT/scripts/build-app.sh"

mkdir -p "$DIST_DIR"

# --- ZIP (preserves resource forks via ditto, the macOS-correct way) ---
ZIP_OUT="$DIST_DIR/${APP_NAME}-${VERSION}.zip"
echo "→ packaging ${ZIP_OUT##*/}"
rm -f "$ZIP_OUT"
ditto -c -k --keepParent "$APP_PATH" "$ZIP_OUT"

# --- DMG with Applications shortcut for drag-to-install -----------------
DMG_OUT="$DIST_DIR/${APP_NAME}-${VERSION}.dmg"
echo "→ packaging ${DMG_OUT##*/}"
rm -f "$DMG_OUT"

STAGING="$(mktemp -d)/${APP_NAME}-dmg"
mkdir -p "$STAGING"
cp -R "$APP_PATH" "$STAGING/"
ln -s /Applications "$STAGING/Applications"

hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$STAGING" \
    -ov \
    -format UDZO \
    "$DMG_OUT" >/dev/null

rm -rf "$STAGING"

echo
echo "✓ artifacts:"
echo "    $DMG_OUT  ($(du -h "$DMG_OUT" | cut -f1))"
echo "    $ZIP_OUT  ($(du -h "$ZIP_OUT" | cut -f1))"
echo
echo "Distribute the .dmg — users double-click, drag Hush.app to Applications, done."
