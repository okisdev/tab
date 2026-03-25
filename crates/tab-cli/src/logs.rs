use anyhow::Result;
use std::process::Command;

use tab_core::logging::{log_dir, log_file};

pub fn show(component: &str, follow: bool, lines: u32) -> Result<()> {
    let dir = log_dir();

    if component == "all" {
        println!("Log directory: {}\n", dir.display());

        let entries = std::fs::read_dir(&dir);
        match entries {
            Ok(entries) => {
                let mut found = false;
                let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                files.sort_by_key(|e| e.file_name());

                for entry in files {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.ends_with(".log") || name.ends_with(".log.old") {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        println!("  {:<25} {}", name, format_size(size));
                        found = true;
                    }
                }

                if !found {
                    println!("  (no log files yet)");
                }
            }
            Err(_) => {
                println!("  (log directory does not exist yet)");
            }
        }

        println!();
        println!("Usage:");
        println!("  tab logs daemon         last 50 lines");
        println!("  tab logs daemon -f      follow/tail");
        println!("  tab logs hook -n 100    last 100 lines");

        return Ok(());
    }

    let path = log_file(component);
    if !path.exists() {
        println!("No log file for '{component}' ({})", path.display());
        return Ok(());
    }

    let mut args = vec![];
    if follow {
        args.push("-f".to_string());
    }
    args.push(format!("-n{lines}"));
    args.push(path.to_string_lossy().to_string());

    let status = Command::new("tail").args(&args).status()?;
    if !status.success() {
        anyhow::bail!("tail exited with {status}");
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
