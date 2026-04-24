use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use tab_core::paths::{log_dir, log_file};

pub fn show(component: &str, follow: bool, lines: u32) -> Result<()> {
    if component == "all" {
        return print_directory_listing();
    }

    let path = log_file(component);
    if !path.exists() {
        println!("No log file for '{component}' ({})", path.display());
        return Ok(());
    }

    print_last_lines(&path, lines as usize)?;

    if follow {
        follow_log(&path)?;
    }

    Ok(())
}

fn print_directory_listing() -> Result<()> {
    let dir = log_dir();
    println!("Log directory: {}\n", dir.display());
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            files.sort_by_key(|e| e.file_name());
            let mut any = false;
            for entry in files {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".log") || name.ends_with(".log.old") {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    println!("  {:<25} {}", name, format_size(size));
                    any = true;
                }
            }
            if !any {
                println!("  (no log files yet)");
            }
        }
        Err(_) => println!("  (log directory does not exist yet)"),
    }
    println!();
    println!("Usage:");
    println!("  tab logs daemon         last 50 lines");
    println!("  tab logs daemon -f      follow/tail");
    println!("  tab logs hook -n 100    last 100 lines");
    Ok(())
}

fn print_last_lines(path: &Path, lines: usize) -> Result<()> {
    let all = std::fs::read_to_string(path)?;
    let collected: Vec<&str> = all.lines().collect();
    let start = collected.len().saturating_sub(lines);
    for line in &collected[start..] {
        println!("{line}");
    }
    Ok(())
}

/// `tail -F` equivalent — survives log rotation by reopening when the file
/// shrinks (rename → new empty file) or disappears.
fn follow_log(path: &Path) -> Result<()> {
    let (mut reader, mut pos) = open_and_seek_end(path)?;

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                std::thread::sleep(Duration::from_millis(250));
                match std::fs::metadata(path) {
                    Ok(meta) => {
                        if meta.len() < pos {
                            // Rotation: old fd is now detached, reopen at 0.
                            let (new_reader, new_pos) = open_and_seek_end(path)?;
                            reader = new_reader;
                            pos = new_pos;
                            eprintln!("tab: log rotated, re-opened");
                        }
                    }
                    Err(_) => {
                        // File vanished — wait for it to reappear.
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            Ok(n) => {
                pos += n as u64;
                print!("{line}");
            }
            Err(e) => {
                eprintln!("tab: log read error: {e}");
                return Ok(());
            }
        }
    }
}

fn open_and_seek_end(path: &Path) -> Result<(BufReader<std::fs::File>, u64)> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let pos = reader.seek(SeekFrom::End(0))?;
    Ok((reader, pos))
}

pub(crate) fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024 - 1), "1024.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }
}
