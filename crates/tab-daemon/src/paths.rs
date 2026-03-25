use std::path::Path;
use tab_core::{Candidate, CandidateSource};

/// Commands that complete with directories only
const DIR_COMMANDS: &[&str] = &["cd", "pushd", "z", "ls"];

/// Commands that complete with files
const FILE_COMMANDS: &[&str] = &[
    "cat", "less", "head", "tail", "vim", "nvim", "nano", "code", "open",
    "rm", "cp", "mv", "chmod", "source", ".", "bat",
    "python", "python3", "node", "ruby", "perl", "bash", "sh", "zsh",
    "cargo", "go", "deno", "bun", "tsx", "ts-node", "npx",
    "touch", "mkdir", "stat", "file", "wc", "grep", "rg", "diff",
];

/// Generate filesystem path candidates based on the current buffer.
pub fn query_paths(buffer: &str, cwd: &str, max_results: usize) -> Vec<Candidate> {
    let (cmd, partial) = match split_command(buffer) {
        Some(v) => v,
        None => return vec![],
    };

    let dirs_only = DIR_COMMANDS.contains(&cmd);
    let files_too = FILE_COMMANDS.contains(&cmd);

    if !dirs_only && !files_too {
        return vec![];
    }

    let base_path = Path::new(cwd);

    // Split partial into directory part and name prefix
    // e.g. "scripts/st" → dir="scripts/", prefix="st"
    // e.g. "scripts/" → dir="scripts/", prefix=""
    // e.g. "st" → dir="", prefix="st"
    let (dir_part, prefix) = if let Some(pos) = partial.rfind('/') {
        (&partial[..=pos], &partial[pos + 1..])
    } else {
        ("", partial)
    };

    let scan_dir = if dir_part.is_empty() {
        base_path.to_path_buf()
    } else {
        base_path.join(dir_part)
    };

    let entries = match std::fs::read_dir(&scan_dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let prefix_lower = prefix.to_ascii_lowercase();
    let mut candidates = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files unless prefix starts with '.'
        if name_str.starts_with('.') && !prefix.starts_with('.') {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        // For cd/z: only directories. For file commands: both.
        if dirs_only && !is_dir {
            continue;
        }

        // Prefix filter
        if !prefix.is_empty()
            && !name_str.to_ascii_lowercase().starts_with(&prefix_lower)
        {
            continue;
        }

        let suffix = if is_dir { "/" } else { "" };
        let full_text = format!("{cmd} {dir_part}{name_str}{suffix}");

        candidates.push(Candidate {
            text: full_text,
            score: 1.0,
            match_positions: vec![],
            source: CandidateSource::Path,
        });
    }

    // Sort alphabetically, then truncate
    candidates.sort_by(|a, b| a.text.cmp(&b.text));
    candidates.truncate(max_results);
    candidates
}

/// Split buffer into (command, partial_path).
/// "cd scripts/st" → Some(("cd", "scripts/st"))
/// "cd " → Some(("cd", ""))
/// "git" → None (no space, still typing command)
fn split_command(buffer: &str) -> Option<(&str, &str)> {
    let first_space = buffer.find(' ')?;
    let cmd = &buffer[..first_space];
    let rest = buffer[first_space + 1..].trim_start();
    // Only complete the last argument (no spaces in the path part)
    if rest.contains(' ') {
        // Multiple args — complete the last one? For now skip.
        return None;
    }
    Some((cmd, rest))
}
