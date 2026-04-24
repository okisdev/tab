use std::path::PathBuf;

use anyhow::Result;

mod templates;

#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(windows)]
mod windows;

pub fn daemon_bin_path() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current_exe has no parent"))?;
    let name = format!("tab-daemon{}", std::env::consts::EXE_SUFFIX);
    Ok(parent.join(name))
}

/// Start the daemon in the foreground (manual debugging).
pub fn start_foreground() -> Result<()> {
    let daemon_bin = daemon_bin_path()?;
    if !daemon_bin.exists() {
        anyhow::bail!("tab-daemon not found at {:?}", daemon_bin);
    }
    let status = std::process::Command::new(&daemon_bin).status()?;
    if !status.success() {
        anyhow::bail!("tab-daemon exited with {status}");
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let running = tab_core::ipc::ping();
    println!(
        "tab-daemon: {}",
        if running { "running" } else { "not running" }
    );

    #[cfg(target_os = "macos")]
    {
        launchd::status()?;
    }
    #[cfg(target_os = "linux")]
    {
        systemd::status()?;
    }
    #[cfg(windows)]
    {
        windows::status()?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        println!("service: not supported on this OS");
    }
    Ok(())
}

pub fn install() -> Result<()> {
    let daemon_bin = daemon_bin_path()?;
    if !daemon_bin.exists() {
        anyhow::bail!(
            "tab-daemon not found at {:?}. Build first with `cargo build --release`.",
            daemon_bin
        );
    }
    if let Err(e) = tab_core::Config::save_default_if_missing() {
        eprintln!("tab: warning — could not write default config: {e}");
    }

    #[cfg(target_os = "macos")]
    {
        launchd::install(&daemon_bin)?;
    }
    #[cfg(target_os = "linux")]
    {
        systemd::install(&daemon_bin)?;
    }
    #[cfg(windows)]
    {
        windows::install(&daemon_bin)?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        anyhow::bail!("service install not supported on this OS");
    }
    print_post_install_hint();
    Ok(())
}

pub fn uninstall() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        launchd::uninstall()?;
    }
    #[cfg(target_os = "linux")]
    {
        systemd::uninstall()?;
    }
    #[cfg(windows)]
    {
        windows::uninstall()?;
    }

    let dir = tab_core::paths::runtime_dir();
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    println!();
    println!("Remove the corresponding line from your shell config (see `tab init <shell>`)");
    Ok(())
}

fn print_post_install_hint() {
    println!();
    println!("Activate tab in your shell:");
    println!(r#"  zsh:         eval "$(tab init zsh)"   # ~/.zshrc"#);
    println!(r#"  bash:        eval "$(tab init bash)"  # ~/.bashrc"#);
    println!(
        r#"  fish:        tab init fish | source   # (persist in ~/.config/fish/conf.d/tab.fish)"#
    );
    println!(r#"  pwsh:        tab init pwsh | Out-String | Invoke-Expression  # $PROFILE"#);
}

pub fn doctor() -> Result<()> {
    println!("tab {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("── Daemon ──");
    match tab_core::ipc::ping() {
        true => println!("  status: running"),
        false => println!("  status: not running  (tab install / tab start)"),
    }
    println!("  socket: {}", tab_core::paths::socket_file().display());
    println!("  logs  : {}", tab_core::paths::log_dir().display());

    println!();
    println!("── Shells detected ──");
    for shell in tab_history::ShellKind::all() {
        let path = tab_core::paths::default_history_path(shell.as_str());
        let (present, path_str) = match path {
            Some(p) => (p.exists(), p.display().to_string()),
            None => (false, "(none)".to_string()),
        };
        println!(
            "  {:<5} {:<9} {}",
            shell.as_str(),
            if present { "present" } else { "missing" },
            path_str
        );
    }

    println!();
    println!("── Binaries ──");
    let daemon = daemon_bin_path()?;
    println!(
        "  tab-daemon: {} {}",
        daemon.display(),
        if daemon.exists() { "[OK]" } else { "[MISSING]" }
    );
    println!(
        "  OS        : {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_bin_sits_next_to_current_exe() {
        let path = daemon_bin_path().expect("daemon_bin_path");
        let exe = std::env::current_exe().unwrap();
        assert_eq!(path.parent(), exe.parent());
        let expected_name = format!("tab-daemon{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some(expected_name.as_str())
        );
    }
}
