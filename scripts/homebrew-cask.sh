#!/bin/sh
# Prints the Homebrew cask for a release, for the yldrmzffr/homebrew-tap repo:
#
#   scripts/homebrew-cask.sh 0.1.4 <sha256 of humon-0.1.4-macos-universal.zip>
#
# The release workflow runs this after publishing and pushes the result to
# Casks/humon.rb in the tap.
set -eu

version=${1:?usage: homebrew-cask.sh VERSION SHA256}
sha256=${2:?usage: homebrew-cask.sh VERSION SHA256}

cat <<EOF
cask "humon" do
  version "$version"
  sha256 "$sha256"

  url "https://github.com/yldrmzffr/humon/releases/download/v#{version}/humon-#{version}-macos-universal.zip"
  name "humon"
  desc "Small, fast MongoDB client"
  homepage "https://github.com/yldrmzffr/humon"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :monterey

  app "humon.app"

  zap trash: "~/Library/Application Support/dev.muzaffer.humon"

  caveats <<~EOS
    humon is not notarized by Apple yet, so macOS blocks its first launch.
    Open System Settings → Privacy & Security and click "Open Anyway", or run:
      xattr -dr com.apple.quarantine #{appdir}/humon.app

    Saved passwords live in the Keychain under "dev.muzaffer.humon";
    \`brew uninstall --zap\` leaves them, remove them in Keychain Access.
  EOS
end
EOF
