use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::source::HistorySource;
use crate::HistoryEntry;

pub struct Pwsh;

impl HistorySource for Pwsh {
    fn parse(path: &Path) -> Result<Vec<HistoryEntry>> {
        let bytes = fs::read(path)?;
        let content = decode_lossy(&bytes);
        Ok(parse_str(&content))
    }
}

/// PSReadLine writes UTF-16 LE with a BOM on Windows, UTF-8 on other OSes.
fn decode_lossy(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// PSReadLine stores one command per logical line; multi-line commands end each
/// continuation line with a backtick (\`).
fn parse_str(content: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut buf = String::new();

    for line in content.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_suffix('`') {
            buf.push_str(rest);
            buf.push('\n');
        } else {
            buf.push_str(line);
            if !buf.trim().is_empty() {
                entries.push(HistoryEntry {
                    command: std::mem::take(&mut buf).trim().to_string(),
                    timestamp: 0,
                    duration: 0,
                });
            } else {
                buf.clear();
            }
        }
    }
    if !buf.trim().is_empty() {
        entries.push(HistoryEntry {
            command: buf.trim().to_string(),
            timestamp: 0,
            duration: 0,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_lines() {
        let entries = parse_str("Get-ChildItem\ncd C:\\temp\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "Get-ChildItem");
        assert_eq!(entries[1].command, "cd C:\\temp");
    }

    #[test]
    fn parse_backtick_continuation() {
        let entries = parse_str("Get-Process `\n  | Where-Object Name -like 'p*'\n");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].command.contains("Get-Process"));
        assert!(entries[0].command.contains("Where-Object"));
    }

    #[test]
    fn skip_blank_lines() {
        let entries = parse_str("ls\n\n\ncd\n");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn utf16_le_bom() {
        let text = "ls\r\ncd\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for c in text.encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        let decoded = decode_lossy(&bytes);
        let entries = parse_str(&decoded);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "ls");
    }

    #[test]
    fn utf16_be_bom() {
        let text = "Get-Process\r\ncd ..\r\n";
        let mut bytes = vec![0xFE, 0xFF];
        for c in text.encode_utf16() {
            bytes.extend_from_slice(&c.to_be_bytes());
        }
        let decoded = decode_lossy(&bytes);
        let entries = parse_str(&decoded);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "Get-Process");
        assert_eq!(entries[1].command, "cd ..");
    }

    #[test]
    fn utf8_without_bom() {
        let entries = parse_str(&decode_lossy(b"Get-ChildItem\n"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "Get-ChildItem");
    }

    #[test]
    fn empty_file_yields_no_entries() {
        assert!(parse_str("").is_empty());
        assert!(parse_str("\n\n\n").is_empty());
    }

    #[test]
    fn backtick_followed_by_blank() {
        // Real-world edge case: user typed a backtick at line end then Enter.
        let entries = parse_str("a `\n");
        // The continuation absorbs the blank — not a crash.
        assert!(entries.iter().all(|e| !e.command.is_empty()));
    }

    #[test]
    fn parse_from_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hist.txt");
        std::fs::write(&path, "ls\ncd /tmp\n").unwrap();
        let entries = Pwsh::parse(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
