cask "hush-dictation" do
  version "0.4.2"
  sha256 "62071c6d96126cb150b117961f242e01484403ee37b1a1aa9f7044e7f33aaa41"

  url "https://github.com/djmunro/hush/releases/download/v#{version}/Hush-#{version}.dmg"
  name "Hush"
  desc "Local push-to-talk dictation for macOS"
  homepage "https://github.com/djmunro/hush"

  depends_on macos: ">= :big_sur"
  depends_on arch: :arm64

  app "Hush.app"

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
    prompted on first launch.

    Homebrew can't remove macOS TCC permissions on uninstall. To fully reset:
      tccutil reset Microphone com.djmunro.hush
      tccutil reset Accessibility com.djmunro.hush
  EOS
end
