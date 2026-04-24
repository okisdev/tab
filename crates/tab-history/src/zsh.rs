use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::source::HistorySource;
use crate::HistoryEntry;

pub struct Zsh;

impl HistorySource for Zsh {
    fn parse(path: &Path) -> Result<Vec<HistoryEntry>> {
        let bytes = fs::read(path)?;
        let content = String::from_utf8_lossy(&bytes);
        Ok(parse_str(&content))
    }
}

fn parse_str(content: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut current_command: Option<String> = None;
    let mut current_ts: i64 = 0;
    let mut current_dur: u32 = 0;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(": ") {
            if let Some(cmd) = current_command.take() {
                push(&mut entries, cmd, current_ts, current_dur);
            }

            if let Some((meta, cmd)) = rest.split_once(';') {
                let parts: Vec<&str> = meta.splitn(2, ':').collect();
                current_ts = parts[0].parse().unwrap_or(0);
                current_dur = parts.get(1).and_then(|d| d.parse().ok()).unwrap_or(0);

                if let Some(stripped) = cmd.strip_suffix('\\') {
                    let mut s = String::with_capacity(stripped.len() + 1);
                    s.push_str(stripped);
                    s.push('\n');
                    current_command = Some(s);
                } else {
                    push(&mut entries, cmd.to_string(), current_ts, current_dur);
                }
            }
        } else if let Some(ref mut cmd) = current_command {
            if let Some(stripped) = line.strip_suffix('\\') {
                cmd.push_str(stripped);
                cmd.push('\n');
            } else {
                cmd.push_str(line);
                let taken = current_command.take().unwrap();
                push(&mut entries, taken, current_ts, current_dur);
            }
        } else if !line.trim().is_empty() {
            // Plain-text history (zsh without EXTENDED_HISTORY)
            entries.push(HistoryEntry {
                command: line.trim().to_string(),
                timestamp: 0,
                duration: 0,
            });
        }
    }

    if let Some(cmd) = current_command {
        push(&mut entries, cmd, current_ts, current_dur);
    }
    entries
}

fn push(entries: &mut Vec<HistoryEntry>, cmd: String, ts: i64, dur: u32) {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return;
    }
    entries.push(HistoryEntry {
        command: trimmed.to_string(),
        timestamp: ts,
        duration: dur,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extended() {
        let input = "\
: 1735960183:0;git status
: 1735960200:1;cargo build
: 1735960220:0;ls -la
";
        let entries = parse_str(input);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].command, "git status");
        assert_eq!(entries[0].timestamp, 1735960183);
    }

    #[test]
    fn parse_multiline() {
        let input = "\
: 1735960183:0;echo hello && \\
echo world
: 1735960200:0;ls
";
        let entries = parse_str(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "echo hello && \necho world");
        assert_eq!(entries[1].command, "ls");
    }

    #[test]
    fn parse_plain() {
        let entries = parse_str("ls -la\ncd /tmp\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "ls -la");
    }

    #[test]
    fn parse_empty() {
        assert!(parse_str("").is_empty());
    }

    #[test]
    fn parse_malformed_header_skips_cleanly() {
        // `:` without `;` is malformed; parser should drop the header but
        // continue with the next valid one.
        let input = "\
: broken_no_semicolon
: 1700000100:0;good
";
        let entries = parse_str(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "good");
    }

    #[test]
    fn parse_from_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zsh_hist");
        std::fs::write(&path, ": 1700000000:0;ls\n").unwrap();
        let entries = Zsh::parse(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "ls");
    }
}
