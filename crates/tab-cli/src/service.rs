use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

const PLIST_LABEL: &str = "com.tab.daemon";

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/LaunchAgents/com.tab.daemon.plist")
}

fn log_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".local/share/tab/logs")
}

fn daemon_bin_path() -> Result<PathBuf> {
    let path = std::env::current_exe()?
        .parent()
        .unwrap()
        .join("tab-daemon");
    Ok(path)
}

/// Start the daemon in the foreground (for manual testing)
pub fn start_foreground() -> Result<()> {
    let daemon_bin = daemon_bin_path()?;
    if !daemon_bin.exists() {
        anyhow::bail!("tab-daemon not found at {:?}", daemon_bin);
    }

    let status = Command::new(&daemon_bin).status()?;
    if !status.success() {
        anyhow::bail!("tab-daemon exited with {status}");
    }
    Ok(())
}

/// Check daemon status
pub fn status() -> Result<()> {
    let shell_sock = tab_core::shell_socket_path();
    if shell_sock.exists() {
        println!("tab-daemon: running (socket: {:?})", shell_sock);
    } else {
        println!("tab-daemon: not running");
    }

    let plist = plist_path();
    if plist.exists() {
        println!("launchd: installed ({:?})", plist);
    } else {
        println!("launchd: not installed");
    }

    Ok(())
}

/// Install: generate launchd plist, load it, print shell setup hint
pub fn install() -> Result<()> {
    let daemon_bin = daemon_bin_path()?;
    if !daemon_bin.exists() {
        anyhow::bail!(
            "tab-daemon not found at {:?}. Build first with `cargo build --release`",
            daemon_bin
        );
    }

    // Create log directory
    let logs = log_dir();
    std::fs::create_dir_all(&logs)?;

    // Generate plist
    let plist_content = generate_plist(&daemon_bin, &logs);
    let plist = plist_path();

    // Ensure LaunchAgents directory exists
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Unload existing if present
    if plist.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist)
            .output();
    }

    std::fs::write(&plist, plist_content)?;
    println!("wrote {:?}", plist);

    // Load the service
    let output = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist)
        .output()?;

    if output.status.success() {
        println!("tab-daemon loaded via launchd");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("launchctl load failed: {stderr}");
    }

    println!();
    println!("Add to your ~/.zshrc:");
    println!("  eval \"$(tab init zsh)\"");
    println!();
    println!("Then restart your shell or run:");
    println!("  source ~/.zshrc");

    Ok(())
}

/// Uninstall: stop daemon, remove plist
pub fn uninstall() -> Result<()> {
    let plist = plist_path();

    if plist.exists() {
        let output = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist)
            .output()?;

        if output.status.success() {
            println!("tab-daemon unloaded from launchd");
        }

        std::fs::remove_file(&plist)?;
        println!("removed {:?}", plist);
    } else {
        println!("launchd plist not found (not installed)");
    }

    // Clean up socket files
    let sock_dir = tab_core::socket_dir();
    if sock_dir.exists() {
        let _ = std::fs::remove_dir_all(&sock_dir);
        println!("cleaned up socket directory");
    }

    println!();
    println!("Remove from your ~/.zshrc:");
    println!("  eval \"$(tab init zsh)\"");

    Ok(())
}

fn generate_plist(daemon_path: &std::path::Path, log_dir: &std::path::Path) -> String {
    let daemon_str = daemon_path.display();
    let log_str = log_dir.display();

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{PLIST_LABEL}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{daemon_str}</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>{log_str}/daemon-crash.log</string>

    <key>StandardErrorPath</key>
    <string>{log_str}/daemon-crash.log</string>

    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#
    )
}
