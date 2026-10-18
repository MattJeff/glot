# Copy to MattJeff/homebrew-glot as Formula/glot.rb, then: brew install mattjeff/glot/glot
# Bump: replace v0.1.0 in the four URLs and their sha256 (gh release view vX.Y.Z -R MattJeff/glot --json assets).
class Glot < Formula
  desc "AI-native i18n for SaaS: CLI, MCP server and deterministic translation validator"
  homepage "https://github.com/MattJeff/glot"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.3.0/glot-aarch64-apple-darwin.tar.gz"
      sha256 "8a8ca946f2a7c4c66eb497d0a3b1823495f5398e3243c0120c33f9fcb7b1b4d1"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.3.0/glot-x86_64-apple-darwin.tar.gz"
      sha256 "dfd56c761854b1b62b96f63d7e087718df41665581543bd4c1bb16a613dfddd0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.3.0/glot-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "6549b28be8a293fca026346386bf291fa7bb7cd6d75f660bf5a8e84d92dc1153"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.3.0/glot-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "8fd9993969472bf8e635b0220561769fcc67544d454da979d279dd344cb1c450"
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
