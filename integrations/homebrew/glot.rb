# Copy to MattJeff/homebrew-glot as Formula/glot.rb, then: brew install mattjeff/glot/glot
# Bump: replace v0.1.0 in the four URLs and their sha256 (gh release view vX.Y.Z -R MattJeff/glot --json assets).
class Glot < Formula
  desc "AI-native i18n for SaaS: CLI, MCP server and deterministic translation validator"
  homepage "https://github.com/MattJeff/glot"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.4.0/glot-aarch64-apple-darwin.tar.gz"
      sha256 "1bf3a8e632e3dcba8b39b8b38e02a411dec51e08230010c16968a6797f60bcfa"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.4.0/glot-x86_64-apple-darwin.tar.gz"
      sha256 "dc34e76076f6088e43e13ae1050e1d5c7d1db086852103a1d73b151b6e6e3067"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/MattJeff/glot/releases/download/v0.4.0/glot-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "301488ce82a100e3d9d227e583ad7e57a5979248fb7557225d187a3e86b09008"
    end
    on_intel do
      url "https://github.com/MattJeff/glot/releases/download/v0.4.0/glot-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "58d275b71b796e0db25a5b652565ec9abaed05d34fa53431b3f1c57713eeb2ce"
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
