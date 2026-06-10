cask "hush-dictation" do
  version "0.4.3"
  sha256 "57395387c631be29628f542f2302057ddd6d9cd2abb4d7ec809e85d0ca251d11"

  url "https://github.com/djmunro/hush/releases/download/v#{version}/Hush-#{version}.dmg"
  name "Hush"
  desc "Local push-to-talk dictation for macOS"
  homepage "https://github.com/djmunro/hush"

  depends_on macos: ">= :big_sur"
  depends_on arch: :arm64

  app "Hush.app"

  # The app is ad-hoc signed (no Developer ID), so Gatekeeper blocks it
  # with "couldn't verify the source" if the quarantine flag survives.
  # We own this tap, so the cask strips it itself.
  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/Hush.app"]
  end

  uninstall launchctl: "com.djmunro.hush",
            quit:      "com.djmunro.hush"

  zap trash: [
    "~/Library/LaunchAgents/com.djmunro.hush.plist",
    "~/Library/Saved Application State/com.djmunro.hush.savedState",
    "~/Library/Preferences/com.djmunro.hush.plist",
    "~/.cache/hush",
  ]

  caveats <<~EOS
    Hush needs Microphone and Accessibility permissions. Grant them when
    prompted on first launch. The quarantine flag is removed on install
    (the app is ad-hoc signed), so no Gatekeeper dialog should appear.

    Homebrew can't remove macOS TCC permissions on uninstall. To fully reset:
      tccutil reset Microphone com.djmunro.hush
      tccutil reset Accessibility com.djmunro.hush
  EOS
end
