#!/bin/bash
# Dev install — builds Hush.app and lands it at /Applications/Hush.app
# (the canonical install path), then opens it.
#
# Why this exists:
#
#   macOS TCC keys permission grants on code-signing identity. For
#   ad-hoc-signed bundles like ours, that identity changes EVERY rebuild
#   (the cdhash is a hash of the binary contents). So every fresh build
#   is a "new app" to TCC, and previous grants do not apply.
#
#   The least-surprising dev workflow:
#     1. Always launch the SAME path (/Applications/Hush.app).
#     2. On each rebuild, reset TCC for hush's bundle ID so prompts
#        re-fire cleanly, instead of silently inheriting Denied state
#        from the previous identity.
#     3. Quit the running instance, swap the bundle, re-open.
#
#   Production fix is real Developer ID signing — TCC then recognizes
#   that build N+1 is the same app as N and preserves grants. Until
#   then, treat every dev install as a fresh start.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUNDLE_ID="com.djmunro.hush"
APP_NAME="Hush"
DEST_APP="/Applications/${APP_NAME}.app"

# 1. Build the bundle.
bash "$ROOT/scripts/build-app.sh"
BUILT_APP="$ROOT/target/release/${APP_NAME}.app"

# 2. Quit any running instance (Hush.app or bare binary).
echo "→ stopping running hush instances"
osascript -e 'tell application "Hush" to quit' 2>/dev/null || true
pkill -f "Hush.app/Contents/MacOS/hush" 2>/dev/null || true
pkill -x hush 2>/dev/null || true
sleep 0.5

# 3. Reset TCC for hush's bundle ID — clears any sticky Denied state
#    from a previous (now-invalidated) cdhash.
echo "→ resetting TCC entries for $BUNDLE_ID"
for svc in Microphone Accessibility ListenEvent; do
    tccutil reset "$svc" "$BUNDLE_ID" >/dev/null 2>&1 || true
done

# 4. Swap the installed bundle.
echo "→ installing to $DEST_APP"
if [[ -d "$DEST_APP" ]]; then
    rm -rf "$DEST_APP"
fi
cp -R "$BUILT_APP" "$DEST_APP"

# 5. Open the freshly installed bundle.
echo "→ launching"
open "$DEST_APP"

cat <<EOF

✓ installed and launched $DEST_APP

What happens now:
  - Hush.app appears in your menubar (top-right).
  - Settings window auto-opens because perms are reset.
  - Click "Allow microphone" → standard system prompt → Allow.
  - Click "Open Input Monitoring…" → toggle Hush ON in System Settings.
  - Click "Open Accessibility…" → toggle Hush ON in System Settings.

Stale entries to clean up MANUALLY in System Settings (one-time):
  Privacy & Security → Microphone / Input Monitoring / Accessibility
    - "hush" with the generic exec icon (old bare-binary)
    - "python3.14" (old osascript misattribution)
  Click each, hit the - button. The NEW Hush.app entry will show its
  proper icon when you grant from the in-app buttons.

Re-running this script later:
  - It will reset perms again. You'll re-grant once.
  - That's the cost of ad-hoc signing. A real Developer ID would let
    grants persist across rebuilds.
EOF
