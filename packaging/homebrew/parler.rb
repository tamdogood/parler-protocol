# Homebrew formula for the Parler Protocol CLI.
#
# This is the ready-to-ship formula for a tap (`brew install tamdogood/tap/parler`). To publish:
#   1. Cut a `vX.Y.Z` tag — the `Release CLI` workflow builds + uploads the prebuilt tarballs.
#   2. Copy this file into the tap repo (`tamdogood/homebrew-tap` → `Formula/parler.rb`).
#   3. Fill the three `sha256` values from the `*.tar.gz.sha256` files on the Release.
#      (`brew bump-formula-pr` automates steps 2–3 on later releases.)
#
# Until the tap exists, the works-today install is `curl -fsSL …/install.sh | sh`.
class Parler < Formula
  desc "One tiny hub so your AI agents can find, verify, and message each other"
  homepage "https://github.com/tamdogood/parler-ai"
  version "0.1.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/tamdogood/parler-ai/releases/download/v#{version}/parler-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_aarch64-apple-darwin_SHA256"
    end
    on_intel do
      url "https://github.com/tamdogood/parler-ai/releases/download/v#{version}/parler-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_x86_64-apple-darwin_SHA256"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/tamdogood/parler-ai/releases/download/v#{version}/parler-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_x86_64-unknown-linux-gnu_SHA256"
    end
  end

  def install
    bin.install "parler"
  end

  test do
    assert_match "parler", shell_output("#{bin}/parler --version")
  end
end
