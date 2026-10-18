# Copy to MattJeff/homebrew-glot as Formula/glot.rb, then: brew install mattjeff/glot/glot
# Bump: replace v0.1.0 in the four URLs and their sha256 (gh release view vX.Y.Z -R MattJeff/glot --json assets).
class Glot < Formula
  desc "AI-native i18n for SaaS: CLI, MCP server and deterministic translation validator"
  homepage "https://github.com/MattJeff/glot"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.0/glot-aarch64-apple-darwin.tar.gz"
      sha256 "0fac628d527179576cafd195a541e50975dffcb83a8ec1fecb221585aa141d76"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.0/glot-x86_64-apple-darwin.tar.gz"
      sha256 "841868db784b07ec842711af657d0143b44bfd9d52dc2d02cbb62c7386eb3546"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.0/glot-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c21c574cbd5be78c0b9b54e9e4be88879553bb3ac0dfb08ab11e49bc2bcb15f3"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.0/glot-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "3f1b6e30611122cca3f0e9da2eab61317a77b308f6ad4e3098a0190817cc327a"
    end
  end

  def install
    bin.install "glot", "glot-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/glot --version")
    (testpath/"package.json").write('{"dependencies":{"next-intl":"*"}}')
    system bin/"glot", "init", "--yes", "--targets", "fr", "--provider", "mock"
    assert_path_exists testpath/"glot.toml"
  end
end
