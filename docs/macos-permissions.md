# macOS permissions, TCC, and code signing — what we learned

Most of hush's complexity is not the dictation itself. It's getting macOS to
let an unsigned local app talk to the microphone, the global event tap, and
the keyboard. This doc is the field guide so future-you doesn't relearn it.

## TCC keys grants on code identity, not name

TCC (Transparency, Consent, and Control) is the database behind System
Settings → Privacy & Security. Every grant is recorded against a *code
identity*, not a binary path or a display name. Identity tiers, in
descending order of stability:

1. **Apple Developer ID** ($99/yr) — TCC matches by team ID + bundle ID +
   designated requirement. Grants persist across rebuilds because the
   signing identity is stable.
2. **Ad-hoc signed bundle** (`codesign --sign -`) — TCC matches by bundle ID
   *and* cdhash. The cdhash is a hash of the binary contents, so **every
   `cargo build` invalidates prior grants**. From TCC's perspective, build
   N+1 is a different app than build N.
3. **Unsigned bare binary** — TCC keys off the file path. Same path can
   sometimes preserve grants, but macOS is increasingly hostile to this and
   the entry shows up with a generic green "exec" icon.

This is why a fresh `cargo build` of an ad-hoc-signed bundle silently loses
its grants every time. The dev workflow has to acknowledge this.

## The `python3.14` saga

Symptom: granting Accessibility to hush popped a system prompt that said
*"python3.14 would like to receive keystrokes"*, and an entry called
`python3.14` appeared in the Accessibility list.

Cause: hush originally pasted text by shelling out to
`osascript -e 'tell application "System Events" to keystroke "v" using command down'`.
macOS attributes Accessibility prompts to the **responsible process** in
the launch tree. If your terminal session was launched from a process tree
whose root was `python3.14` (e.g. a `uv run` shell, an IDE that runs as a
Python app), `osascript` inherits that responsibility and TCC asks
permission for *python3.14*, not hush.

Fix: `src/keyboard.rs` posts Cmd+V via `CGEventPost` directly. No subprocess,
no inherited responsibility. Permission attribution lands on hush itself.

**Rule: never shell out to `osascript` for keystrokes.**

## The hardened-runtime trap

The hardened runtime (`codesign --options runtime`) gates microphone,
Accessibility, and other TCC-protected APIs on **explicit entitlements**
declared in a signed `Entitlements.plist` (e.g.
`com.apple.security.device.audio-input`). If those entitlements are absent,
macOS denies the request **at the kernel level, silently — no TCC prompt
fires and no entry is added to the System Settings list**.

We don't ship entitlements in our ad-hoc dev bundle, so the runtime flag
poisoned every permission ask. Removing it makes prompts fire normally for
local dev. Production / notarized distribution needs both runtime AND a
signed entitlements file.

See `scripts/build-app.sh` — we deliberately omit `--options runtime`.

If you ever notarize for distribution, you'll need an `Entitlements.plist`
with at minimum:

```xml
<key>com.apple.security.device.audio-input</key><true/>
<key>com.apple.security.device.microphone</key><true/>
```

…and pass `--entitlements path/to/Entitlements.plist --options runtime` to
codesign.

## Pick the right API per service

| Service | Preflight | Request prompt | Notes |
|---------|-----------|----------------|-------|
| Microphone | `AVCaptureDevice::authorizationStatusForMediaType(AVMediaTypeAudio)` | `AVCaptureDevice::requestAccessForMediaType_completionHandler` | In-app popup, no System Settings detour |
| Accessibility | `AXIsProcessTrusted` | `AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: true})` | Canonical register-and-prompt; `CGRequestPostEventAccess` can silently no-op |

`CGRequestPostEventAccess` exists, but in practice it can register the
binary in the TCC database without ever surfacing the entry in System
Settings → Accessibility. `AXIsProcessTrustedWithOptions(prompt:true)` is
the API that reliably adds the entry to the list.

## Input Monitoring: do not use it. NSEvent.addGlobalMonitor under Accessibility is enough.

We originally used `CGEventTap` (with `CGEventTapOptions::ListenOnly`) to
detect global fn-key press/release. That requires the separate **Input
Monitoring** TCC permission (`kTCCServiceListenEvent`). It also has a
brutal failure mode on ad-hoc-signed dev builds: `CGPreflightListenEventAccess`
checks the bundle ID against the TCC database, but `CGEventTapCreate` does
a live cdhash check against the kernel. When the two disagree (which they
do every time `cargo build` produces a new cdhash), `CGEventTapCreate`
silently re-fires the *"Keystroke Receiving"* TCC prompt every time it's
called — a prompt loop the user can't escape from if you retry on a timer.

The fix isn't a better retry policy. It's not using `CGEventTap` for this
at all. Replace it with `NSEvent.addGlobalMonitorForEventsMatchingMask`
matching `NSEventMask::FlagsChanged`. That API:

1. Detects global modifier-key state changes (including fn) the same as
   `CGEventTap`'s `FlagsChanged`.
2. Is gated only on **Accessibility** — no separate Input Monitoring
   prompt, no separate Settings entry, no separate cdhash check.
3. Doesn't fire any TCC prompt itself. It silently no-ops without
   permission and starts delivering events the moment Accessibility is
   granted — no reinstall on grant needed.
4. Is what Wispr Flow uses (per their MDM deployment profile, which lists
   Accessibility + Microphone only — no `kTCCServiceListenEvent`).

The receipt: Apple DTS engineer Quinn the Eskimo on the developer forums
states explicitly that for global event monitoring, "the former [NSEvent
global monitor] requires the **Accessibility privilege** whereas the
latter [`CGEventTap`] requires the **Input Monitoring privilege**."
([Apple Forums #707680](https://developer.apple.com/forums/thread/707680))

Implementation: `src/main.rs::install_fn_monitor`. Block fires on the
main thread, edge-detects on `event.modifierFlags().contains(.Function)`,
sends `Msg::Start` / `Msg::Stop` over the worker channel.

**Rule: never reintroduce `CGEventTap` for global key listening. Use
`NSEvent.addGlobalMonitor` instead.**

## TCC has no notification API

When the user toggles a permission in System Settings, your app gets no
callback. The only way to detect the change is to poll. We use an NSTimer
at 1.5s in `src/ui.rs` plus `NSApplicationDidBecomeActiveNotification` and
`NSWindowDidBecomeKeyNotification` observers. The combination feels
near-instant in practice.

## The dev TCC reset workflow

Because ad-hoc cdhash changes invalidate grants every build, the only sane
local dev loop is:

1. **One canonical install path** — `/Applications/Hush.app`. Always launch
   from there. Never launch from `target/release/Hush.app` directly.
2. **Reset TCC for hush's bundle ID on every reinstall** — `tccutil reset
   <Service> com.djmunro.hush` clears the sticky Denied state from the
   prior cdhash so prompts re-fire cleanly.
3. **Quit running, copy bundle, re-open** — atomic swap.

`scripts/install-dev.sh` automates all three. Just `bash
scripts/install-dev.sh` after any code change.

For users on a release DMG, none of this matters: they install once and
permissions stick because the bundle is never overwritten.

## App bundle requirements

Minimum `Info.plist` for a menubar accessory:

```xml
<key>CFBundleIdentifier</key>            <string>com.djmunro.hush</string>
<key>CFBundleExecutable</key>            <string>hush</string>
<key>CFBundleIconFile</key>              <string>AppIcon</string>
<key>CFBundlePackageType</key>           <string>APPL</string>
<key>NSPrincipalClass</key>              <string>NSApplication</string>
<key>LSUIElement</key>                   <true/>           <!-- no dock icon -->
<key>NSMicrophoneUsageDescription</key>  <string>...</string>
<key>NSHighResolutionCapable</key>       <true/>
```

`NSMicrophoneUsageDescription` is *mandatory* for the mic prompt to fire.
If it's missing, `AVCaptureDevice::requestAccessForMediaType` crashes the
app with `[NSException: This app has crashed because it attempted to access
privacy-sensitive data without a usage description...]`.

`LSUIElement=true` makes it a menubar-only accessory: no dock icon, no
appearance in cmd-tab. The user installs to "Login Items" via System
Settings → General → Login Items rather than via a LaunchAgent.

## Cleaning up stale TCC entries

If you see `python3.14`, `hush` (with a generic exec icon), or duplicate
entries in System Settings → Privacy & Security:

```bash
tccutil reset Microphone     com.djmunro.hush
tccutil reset Accessibility  com.djmunro.hush
tccutil reset ListenEvent    com.djmunro.hush   # legacy from older builds
```

Current builds only request Microphone + Accessibility, so any
`com.djmunro.hush` entry under Input Monitoring is safe to remove — it's
left over from when we used `CGEventTap`. The `uninstall.sh` script
includes the `ListenEvent` reset for cleanup of those legacy grants.

For entries that aren't keyed to `com.djmunro.hush` (the bare-binary
leftovers, the python3.14 entry), you can't remove them with `tccutil` —
delete them manually in System Settings with the `−` button.

## Stale processes will gaslight you

When iterating, *make sure no old `hush` process is running before you
launch the new build*. An older process holds the OLD cdhash registered
with TCC; if it's still alive when you grant the new bundle, the old
process can re-fire prompts in the background and make it look like the
new bundle is broken.

`scripts/install-dev.sh` handles this with `pkill -9` against every
plausible hush process name + path, then waits up to 3s for them to
exit before swapping the bundle. If you launch by hand for some reason,
do the same: `pkill -9 -f hush` and check `pgrep -f hush` is empty
before re-opening.

## Notarization (future, not done)

For real distribution outside the developer's machine you need:

1. An Apple Developer Program account ($99/yr).
2. Codesign with a Developer ID Application certificate, hardened runtime,
   and a signed `Entitlements.plist`.
3. Upload to Apple's notary service (`xcrun notarytool submit ...`).
4. Staple the ticket (`xcrun stapler staple ...`).
5. Distribute via DMG.

After notarization, TCC grants persist across rebuilds (the team identity
is stable), Gatekeeper accepts the bundle without warnings, and end users
don't see any "from an unidentified developer" dialogs.

Until then, distribute via the unsigned DMG and tell users to right-click →
Open the first time to bypass Gatekeeper.

## Sources

- [Apple Developer Forums — TCC permissions on macOS](https://developer.apple.com/forums/thread/730043)
- [Apple Developer Forums — How to remove executable applications from TCC](https://developer.apple.com/forums/thread/697278)
- [Jamf — Resetting TCC Prompts on macOS](https://docs.jamf.com/technical-articles/Resetting_Transparency_Consent_and_Control_Prompts_on_macOS.html)
- [SS64 — `tccutil` command reference](https://ss64.com/mac/tccutil.html)
