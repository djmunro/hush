#!/bin/bash
# Builds Hush.app — proper macOS application bundle around the hush
# binary. Solves: generic exec icon in System Settings, "python3.14"
# misattribution risk, unstable TCC entries (bundle ID is stable across
# rebuilds, while bare-binary paths regenerate on every cargo build).
#
# Output: target/release/Hush.app
#
# Why no `cargo bundle`? It's lightly maintained, requires extra Cargo
# metadata, and we already need a script for icon generation. Direct
# bash is shorter than configuring a third-party tool.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUILD_PROFILE="${BUILD_PROFILE:-release}"
BUNDLE_ID="com.djmunro.hush"
APP_NAME="Hush"
DISPLAY_NAME="hush"

if [[ "$BUILD_PROFILE" == "release" ]]; then
    cargo build --release
    BIN="$ROOT/target/release/hush"
    APP_DIR="$ROOT/target/release/${APP_NAME}.app"
else
    cargo build
    BIN="$ROOT/target/debug/hush"
    APP_DIR="$ROOT/target/debug/${APP_NAME}.app"
fi

if [[ ! -x "$BIN" ]]; then
    echo "error: cargo did not produce $BIN" >&2
    exit 1
fi

# --- Bundle skeleton ----------------------------------------------------
echo "→ assembling ${APP_NAME}.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN" "$APP_DIR/Contents/MacOS/hush"
chmod +x "$APP_DIR/Contents/MacOS/hush"

# --- App icon -----------------------------------------------------------
# Generates a 1024×1024 PNG via a Swift script, then iconutil
# transforms a .iconset directory into AppIcon.icns. iconutil is part of
# Xcode CLI tools; sips and Swift ship with macOS.
echo "→ rendering app icon"
TMP_ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$TMP_ICONSET"

SRC_PNG="$TMP_ICONSET/icon_1024.png"
swift "$ROOT/scripts/draw-icon.swift" "$SRC_PNG" >/dev/null

# Apple's required iconset sizes.
sips -z 16 16     "$SRC_PNG" --out "$TMP_ICONSET/icon_16x16.png"      >/dev/null
sips -z 32 32     "$SRC_PNG" --out "$TMP_ICONSET/icon_16x16@2x.png"   >/dev/null
sips -z 32 32     "$SRC_PNG" --out "$TMP_ICONSET/icon_32x32.png"      >/dev/null
sips -z 64 64     "$SRC_PNG" --out "$TMP_ICONSET/icon_32x32@2x.png"   >/dev/null
sips -z 128 128   "$SRC_PNG" --out "$TMP_ICONSET/icon_128x128.png"    >/dev/null
sips -z 256 256   "$SRC_PNG" --out "$TMP_ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256   "$SRC_PNG" --out "$TMP_ICONSET/icon_256x256.png"    >/dev/null
sips -z 512 512   "$SRC_PNG" --out "$TMP_ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512   "$SRC_PNG" --out "$TMP_ICONSET/icon_512x512.png"    >/dev/null
cp                "$SRC_PNG"        "$TMP_ICONSET/icon_512x512@2x.png"
rm "$SRC_PNG"

iconutil -c icns "$TMP_ICONSET" -o "$APP_DIR/Contents/Resources/AppIcon.icns"
rm -rf "$TMP_ICONSET"

# --- Info.plist ---------------------------------------------------------
# LSUIElement=true → no dock icon (menubar accessory app, like ours).
# NSMicrophoneUsageDescription is required for the system mic prompt.
# NSPrincipalClass=NSApplication is required for AppKit apps.
cat > "$APP_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>${DISPLAY_NAME}</string>
    <key>CFBundleExecutable</key>
    <string>hush</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>hush transcribes the audio you record while holding fn. Audio never leaves your machine.</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
EOF

# --- Ad-hoc sign --------------------------------------------------------
# Ad-hoc signing (-s -) gives the bundle a stable code identity tied to
# its bundle ID, which TCC uses to associate permissions.
#
# We deliberately do NOT pass --options runtime (the hardened runtime).
# The hardened runtime gates microphone/Accessibility access on
# explicit entitlements (com.apple.security.device.audio-input etc.),
# and silently denies the request — without ever firing a prompt — when
# those entitlements are absent. For ad-hoc local dev we don't ship
# entitlements, so leave hardened runtime off. (Production / notarized
# distribution would need both: the runtime flag AND an Entitlements
# plist signed alongside.)
echo "→ ad-hoc signing"
codesign --sign - --force --deep "$APP_DIR" >/dev/null

echo "✓ ${APP_DIR}"
