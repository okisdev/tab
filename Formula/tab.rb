class Tab < Formula
  desc "Terminal autocomplete plugin with fuzzy history matching"
  homepage "https://github.com/okisdev/tab"
  version "0.1.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/okisdev/tab/releases/download/v#{version}/tab_v#{version}_darwin_arm64.tar.gz"
      sha256 "PLACEHOLDER"
    end
    if Hardware::CPU.intel?
      url "https://github.com/okisdev/tab/releases/download/v#{version}/tab_v#{version}_darwin_amd64.tar.gz"
      sha256 "PLACEHOLDER"
    end
  end

  def install
    bin.install "tab"
    bin.install "tab-daemon"
    bin.install "tab-overlay"
  end

  def caveats
    <<~EOS
      To activate tab, add to your ~/.zshrc:
        eval "$(tab init zsh)"

      Then install the background service:
        tab install

      Tab requires Accessibility permissions for the overlay popup.
      Grant access in: System Settings → Privacy & Security → Accessibility
    EOS
  end

  service do
    run [opt_bin/"tab-daemon"]
    keep_alive true
    log_path var/"log/tab/daemon.log"
    error_log_path var/"log/tab/daemon.err.log"
  end

  test do
    assert_match "tab", shell_output("#{bin}/tab --help")
  end
end
