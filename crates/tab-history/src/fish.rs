use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::source::HistorySource;
use crate::HistoryEntry;

pub struct Fish;

impl HistorySource for Fish {
    fn parse(path: &Path) -> Result<Vec<HistoryEntry>> {
        let bytes = fs::read(path)?;
        let content = String::from_utf8_lossy(&bytes);
        Ok(parse_str(&content))
    }
}

/// Parse fish's YAML-ish history format.
///
/// Each record:
/// ```yaml
/// - cmd: <command-with-fish-escapes>
///   when: <unix-timestamp>
///   paths:
///     - <path>
/// ```
fn parse_str(content: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut cur_cmd: Option<String> = None;
    let mut cur_ts: i64 = 0;

    let flush = |entries: &mut Vec<HistoryEntry>, cmd: &mut Option<String>, ts: &mut i64| {
        if let Some(c) = cmd.take() {
            let trimmed = c.trim();
            if !trimmed.is_empty() {
                entries.push(HistoryEntry {
                    command: trimmed.to_string(),
                    timestamp: *ts,
                    duration: 0,
                });
            }
        }
        *ts = 0;
    };

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("- cmd: ") {
            flush(&mut entries, &mut cur_cmd, &mut cur_ts);
            cur_cmd = Some(unescape_fish(rest));
        } else if let Some(rest) = line.strip_prefix("  when: ") {
            cur_ts = rest.trim().parse().unwrap_or(0);
        } else {
            // paths/other YAML fields — ignore
        }
    }
    flush(&mut entries, &mut cur_cmd, &mut cur_ts);

    entries
}

/// Fish escapes inside `cmd: ...`:
/// - `\\` → `\`
/// - `\n` → LF
/// - `\t` → tab
fn unescape_fish(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let input = "- cmd: ls -la\n  when: 1735960183\n- cmd: cd /tmp\n  when: 1735960200\n";
        let entries = parse_str(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "ls -la");
        assert_eq!(entries[0].timestamp, 1735960183);
        assert_eq!(entries[1].command, "cd /tmp");
    }

    #[test]
    fn parse_escapes() {
        let input = "- cmd: echo \\\\n world\n  when: 100\n";
        let entries = parse_str(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "echo \\n world");
    }

    #[test]
    fn unescape_fish_all_cases() {
        assert_eq!(unescape_fish(r"plain"), "plain");
        assert_eq!(unescape_fish(r"a\\b"), "a\\b");
        assert_eq!(unescape_fish(r"a\nb"), "a\nb");
        assert_eq!(unescape_fish(r"a\tb"), "a\tb");
        assert_eq!(unescape_fish(r"a\rb"), "a\rb");
        // Unknown escape is preserved verbatim (both chars).
        assert_eq!(unescape_fish(r"a\xb"), r"a\xb");
        // Trailing lone backslash survives.
        assert_eq!(unescape_fish(r"a\"), r"a\");
    }

    #[test]
    fn parse_empty_file() {
        assert!(parse_str("").is_empty());
    }

    #[test]
    fn parse_from_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fish_history");
        std::fs::write(&path, "- cmd: ls\n  when: 100\n").unwrap();
        let entries = Fish::parse(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "ls");
    }

    #[test]
    fn parse_with_paths_section() {
        let input = "\
- cmd: vim foo
  when: 100
  paths:
    - foo
- cmd: ls
  when: 200
";
        let entries = parse_str(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "vim foo");
        assert_eq!(entries[1].command, "ls");
    }
}
