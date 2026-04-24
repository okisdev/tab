use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::service::templates;

pub const LABEL: &str = "com.tab.daemon";

pub fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/LaunchAgents/com.tab.daemon.plist")
}

pub fn install(daemon_bin: &Path) -> Result<()> {
    let logs = tab_core::paths::log_dir();
    std::fs::create_dir_all(&logs)?;

    let plist = plist_path();
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if plist.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist)
            .output();
    }

    std::fs::write(&plist, templates::launchd_plist(LABEL, daemon_bin, &logs))?;
    println!("wrote {}", plist.display());

    let out = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist)
        .output()?;
    if out.status.success() {
        println!("tab-daemon loaded via launchd");
    } else {
        eprintln!(
            "launchctl load failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let plist = plist_path();
    if plist.exists() {
        let out = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist)
            .output()?;
        if out.status.success() {
            println!("tab-daemon unloaded from launchd");
        }
        std::fs::remove_file(&plist)?;
        println!("removed {}", plist.display());
    } else {
        println!("launchd plist not found (not installed)");
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let plist = plist_path();
    if plist.exists() {
        println!("launchd  : installed ({})", plist.display());
    } else {
        println!("launchd  : not installed");
    }
    Ok(())
}
