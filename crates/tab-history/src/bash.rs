use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::source::HistorySource;
use crate::HistoryEntry;

pub struct Bash;

impl HistorySource for Bash {
    fn parse(path: &Path) -> Result<Vec<HistoryEntry>> {
        let bytes = fs::read(path)?;
        let content = String::from_utf8_lossy(&bytes);
        Ok(parse_str(&content))
    }
}

fn parse_str(content: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut pending_ts: Option<i64> = None;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix('#') {
            if let Ok(ts) = rest.trim().parse::<i64>() {
                pending_ts = Some(ts);
                continue;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        entries.push(HistoryEntry {
            command: line.to_string(),
            timestamp: pending_ts.take().unwrap_or(0),
            duration: 0,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain() {
        let entries = parse_str("ls\ncd /tmp\ngit status\n");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].command, "ls");
        assert_eq!(entries[2].command, "git status");
    }

    #[test]
    fn parse_with_timestamps() {
        let entries = parse_str("#1735960183\nls\n#1735960200\ncd /tmp\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp, 1735960183);
        assert_eq!(entries[1].timestamp, 1735960200);
    }

    #[test]
    fn ignore_non_numeric_hash_lines() {
        let entries = parse_str("# my comment\nls\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "# my comment");
    }

    #[test]
    fn blank_lines_are_dropped() {
        let entries = parse_str("\nls\n\n\ncd\n");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn timestamp_only_consumed_by_next_command() {
        let entries = parse_str("#1700000000\nls\ncd\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].timestamp, 1700000000);
        // Second command has no timestamp preceding it.
        assert_eq!(entries[1].timestamp, 0);
    }

    #[test]
    fn parse_from_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bash_hist");
        std::fs::write(&path, "ls\ncd\n").unwrap();
        let entries = Bash::parse(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
