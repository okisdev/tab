use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::service::templates;

fn startup_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Microsoft/Windows/Start Menu/Programs/Startup"))
}

fn startup_file() -> PathBuf {
    startup_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tab-daemon.vbs")
}

pub fn install(daemon_bin: &Path) -> Result<()> {
    let vbs = startup_file();
    if let Some(parent) = vbs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&vbs, templates::windows_vbs(daemon_bin))?;
    println!("wrote {}", vbs.display());

    spawn_detached(daemon_bin)?;
    println!("tab-daemon started (auto-starts on next login)");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let vbs = startup_file();
    if vbs.exists() {
        std::fs::remove_file(&vbs)?;
        println!("removed {}", vbs.display());
    } else {
        println!("startup shortcut not present (not installed)");
    }
    use std::process::Stdio;
    let _ = Command::new("taskkill")
        .args(["/IM", "tab-daemon.exe", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

pub fn status() -> Result<()> {
    let vbs = startup_file();
    if vbs.exists() {
        println!("startup  : installed ({})", vbs.display());
    } else {
        println!("startup  : not installed");
    }
    Ok(())
}

fn spawn_detached(daemon_bin: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(daemon_bin)
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}
