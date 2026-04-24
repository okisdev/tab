use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::service::templates;

pub const UNIT_NAME: &str = "tab.service";

pub fn unit_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("systemd/user")
        .join(UNIT_NAME)
}

pub fn install(daemon_bin: &Path) -> Result<()> {
    let unit = unit_path();
    if let Some(parent) = unit.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&unit, templates::systemd_unit(daemon_bin))?;
    println!("wrote {}", unit.display());

    match Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("systemctl daemon-reload exited with {s}"),
        Err(e) => {
            eprintln!("systemctl not available ({e}); starting daemon in the background");
            spawn_detached(daemon_bin)?;
            return Ok(());
        }
    }

    let _ = Command::new("systemctl")
        .args(["--user", "enable", UNIT_NAME])
        .status();

    let out = Command::new("systemctl")
        .args(["--user", "restart", UNIT_NAME])
        .status()?;
    if out.success() {
        println!("tab-daemon started via systemd (user unit)");
    } else {
        eprintln!("systemctl restart failed ({out}); falling back to spawn");
        spawn_detached(daemon_bin)?;
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let _ = Command::new("systemctl")
        .args(["--user", "stop", UNIT_NAME])
        .status();
    let _ = Command::new("systemctl")
        .args(["--user", "disable", UNIT_NAME])
        .status();

    let unit = unit_path();
    if unit.exists() {
        std::fs::remove_file(&unit)?;
        println!("removed {}", unit.display());
    }
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

pub fn status() -> Result<()> {
    let unit = unit_path();
    if unit.exists() {
        println!("systemd  : unit installed ({})", unit.display());
        let out = Command::new("systemctl")
            .args(["--user", "is-active", UNIT_NAME])
            .output()
            .ok();
        if let Some(o) = out {
            let s = String::from_utf8_lossy(&o.stdout);
            print!("           is-active: {s}");
        }
    } else {
        println!("systemd  : unit not installed");
    }
    Ok(())
}

fn spawn_detached(daemon_bin: &Path) -> Result<()> {
    use std::process::Stdio;
    Command::new(daemon_bin)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    println!("spawned tab-daemon (pid managed by OS, not by systemd)");
    Ok(())
}
