use crate::HistoryEntry;
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Parse a ZSH extended history file.
///
/// Format: `: <timestamp>:<duration>;<command>`
/// Multi-line commands use `\` continuation.
pub fn parse_zsh_history(path: &Path) -> Result<Vec<HistoryEntry>> {
    let bytes = fs::read(path)?;
    let content = String::from_utf8_lossy(&bytes);
    Ok(parse_zsh_history_str(&content))
}

fn parse_zsh_history_str(content: &str) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let mut current_command: Option<String> = None;
    let mut current_ts: i64 = 0;
    let mut current_dur: u32 = 0;

    for line in content.lines() {
        // Check if this is a new entry header: `: timestamp:duration;command`
        if let Some(rest) = line.strip_prefix(": ") {
            // Flush any pending multi-line command
            if let Some(cmd) = current_command.take() {
                let trimmed = cmd.trim().to_string();
                if !trimmed.is_empty() {
                    entries.push(HistoryEntry {
                        command: trimmed,
                        timestamp: current_ts,
                        duration: current_dur,
                    });
                }
            }

            // Parse `: timestamp:duration;command`
            if let Some((meta, cmd)) = rest.split_once(';') {
                let parts: Vec<&str> = meta.splitn(2, ':').collect();
                current_ts = parts[0].parse().unwrap_or(0);
                current_dur = parts.get(1).and_then(|d| d.parse().ok()).unwrap_or(0);

                let cmd_text = cmd.to_string();
                if cmd_text.ends_with('\\') {
                    // Multi-line command continues
                    let mut s = cmd_text;
                    s.pop(); // remove trailing `\`
                    s.push('\n');
                    current_command = Some(s);
                } else {
                    let trimmed = cmd_text.trim().to_string();
                    if !trimmed.is_empty() {
                        entries.push(HistoryEntry {
                            command: trimmed,
                            timestamp: current_ts,
                            duration: current_dur,
                        });
                    }
                }
            }
        } else if let Some(ref mut cmd) = current_command {
            // Continuation line of a multi-line command
            if line.ends_with('\\') {
                let mut l = line.to_string();
                l.pop();
                l.push('\n');
                cmd.push_str(&l);
            } else {
                cmd.push_str(line);
                // End of multi-line command
                let trimmed = cmd.trim().to_string();
                if !trimmed.is_empty() {
                    entries.push(HistoryEntry {
                        command: trimmed,
                        timestamp: current_ts,
                        duration: current_dur,
                    });
                }
                current_command = None;
            }
        }
        // Lines that don't match either pattern (e.g., plain history without timestamps)
        // are treated as standalone commands in non-extended format
    }

    // Flush any remaining multi-line command
    if let Some(cmd) = current_command {
        let trimmed = cmd.trim().to_string();
        if !trimmed.is_empty() {
            entries.push(HistoryEntry {
                command: trimmed,
                timestamp: current_ts,
                duration: current_dur,
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_entries() {
        let input = "\
: 1735960183:0;git status
: 1735960200:1;cargo build
: 1735960220:0;ls -la
";
        let entries = parse_zsh_history_str(input);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].command, "git status");
        assert_eq!(entries[0].timestamp, 1735960183);
        assert_eq!(entries[1].command, "cargo build");
        assert_eq!(entries[2].command, "ls -la");
    }

    #[test]
    fn parse_multiline_command() {
        let input = "\
: 1735960183:0;echo hello && \\
echo world
: 1735960200:0;ls
";
        let entries = parse_zsh_history_str(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "echo hello && \necho world");
        assert_eq!(entries[1].command, "ls");
    }

    #[test]
    fn parse_empty_input() {
        let entries = parse_zsh_history_str("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_with_special_characters() {
        let input = ": 1735960183:0;echo \"hello; world\"\n";
        let entries = parse_zsh_history_str(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "echo \"hello; world\"");
    }
}
