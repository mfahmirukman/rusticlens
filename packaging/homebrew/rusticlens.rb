class Rusticlens < Formula
  desc "Native Rust Kubernetes IDE"
  homepage "https://github.com/mfahmirukman/rusticlens"
  url "https://github.com/mfahmirukman/rusticlens/archive/refs/tags/v0.5.8.tar.gz"
  sha256 "" # fill when tagging release
  license "MIT"
  head "https://github.com/mfahmirukman/rusticlens.git", branch: "develop"

  depends_on "rust" => :build
  depends_on "kubectl"

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/rl-app", root: libexec)
    bin.write_exec_script libexec/"bin/rusticlens", "rusticlens"
  end

  test do
    assert_match "rusticlens", shell_output("#{bin}/rusticlens --help", 2)
  end
end
