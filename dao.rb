class Dao < Formula
  desc "AI-powered software engineering assistant"
  homepage "https://github.com/ShaileshRawat1403/dao"
  version "0.1.2"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/ShaileshRawat1403/dao/releases/download/v0.1.2/dao-cli-v0.1.2-aarch64-apple-darwin.tar.gz"
    sha256 "REPLACE_WITH_SHA256_ARM64"
  elsif OS.mac? && Hardware::CPU.intel?
    url "https://github.com/ShaileshRawat1403/dao/releases/download/v0.1.2/dao-cli-v0.1.2-x86_64-apple-darwin.tar.gz"
    sha256 "REPLACE_WITH_SHA256_X86_64"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/ShaileshRawat1403/dao/releases/download/v0.1.2/dao-cli-v0.1.2-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "REPLACE_WITH_SHA256_LINUX"
  end

  def install
    bin.install "dao"
  end

  test do
    system "#{bin}/dao", "--version"
  end
end
