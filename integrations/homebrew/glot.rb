# Copy to MattJeff/homebrew-glot as Formula/glot.rb, then: brew install mattjeff/glot/glot
# Bump: replace v0.1.0 in the four URLs and their sha256 (gh release view vX.Y.Z -R MattJeff/glot --json assets).
class Glot < Formula
  desc "AI-native i18n for SaaS: CLI, MCP server and deterministic translation validator"
  homepage "https://github.com/MattJeff/glot"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.1.0/glot-aarch64-apple-darwin.tar.gz"
      sha256 "9c41ea372b0d3b1794713c9112fd0619c8d1d6af7a5e06d988908e8881149d0b"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.1.0/glot-x86_64-apple-darwin.tar.gz"
      sha256 "6a3b688cccecc40f62f9dbb21b2543a27ae2449507b4a4e939ea2a6973fcf39e"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.1.0/glot-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "a13f0ef316c245bf92c76dcb99ea261a3319a4e9a30d230ee957d4c58fa25cf5"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.1.0/glot-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "4911d770a830cf78947b161c7f3fcff87119acc729b261efe8c821e0be853b74"
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
