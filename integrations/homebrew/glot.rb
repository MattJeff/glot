# Copy to MattJeff/homebrew-glot as Formula/glot.rb, then: brew install mattjeff/glot/glot
# Bump: replace v0.1.0 in the four URLs and their sha256 (gh release view vX.Y.Z -R MattJeff/glot --json assets).
class Glot < Formula
  desc "AI-native i18n for SaaS: CLI, MCP server and deterministic translation validator"
  homepage "https://github.com/MattJeff/glot"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.1/glot-aarch64-apple-darwin.tar.gz"
      sha256 "e827f7f8a3f58a050769d6dc54faf1deab8a933b3e295bed7eb125a29985e411"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.1/glot-x86_64-apple-darwin.tar.gz"
      sha256 "c43ac61403bda54bdfcbf8ed0e8fc49fed5af9b34b4d6096b8f5f59e8ae86cfa"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.1/glot-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c2bee6f1bc706d0c850994010432614f4e276b2dcafd1a1211092d113c986fe2"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.2.1/glot-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d86f58bfc2a4cef80fa558fb0ccd05b309161be2690a723d2db9a0727e828864"
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
