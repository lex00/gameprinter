class Gp < Formula
  desc "Game Printer tool: port old games, make themes and new games, play in a browser"
  homepage "https://lex00.github.io/gameprinter/"
  version "{{version}}"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "{{url:gp-aarch64-apple-darwin.tar.gz}}"
      sha256 "{{sha256:gp-aarch64-apple-darwin.tar.gz}}"
    end
    on_intel do
      url "{{url:gp-x86_64-apple-darwin.tar.gz}}"
      sha256 "{{sha256:gp-x86_64-apple-darwin.tar.gz}}"
    end
  end

  on_linux do
    on_arm do
      url "{{url:gp-aarch64-unknown-linux-gnu.tar.gz}}"
      sha256 "{{sha256:gp-aarch64-unknown-linux-gnu.tar.gz}}"
    end
    on_intel do
      url "{{url:gp-x86_64-unknown-linux-gnu.tar.gz}}"
      sha256 "{{sha256:gp-x86_64-unknown-linux-gnu.tar.gz}}"
    end
  end

  def install
    bin.install "gp"
    doc.install "README.txt", "THIRD-PARTY.md"
  end

  test do
    assert_match "gp #{version}", shell_output("#{bin}/gp --version")
  end
end
