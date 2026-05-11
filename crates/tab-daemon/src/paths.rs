use std::path::Path;
use tab_core::{Candidate, CandidateSource};

const DIR_COMMANDS: &[&str] = &["cd", "pushd", "z", "ls"];

const FILE_COMMANDS: &[&str] = &[
    "cat", "less", "head", "tail", "vim", "nvim", "nano", "code", "open", "rm", "cp", "mv",
    "chmod", "source", ".", "bat", "python", "python3", "node", "ruby", "perl", "bash", "sh",
    "zsh", "cargo", "go", "deno", "bun", "tsx", "ts-node", "npx", "touch", "mkdir", "stat", "file",
    "wc", "grep", "rg", "diff",
];

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

    // Only treat `\` as a path separator on Windows; elsewhere it's a legal
    // filename character.
    #[cfg(windows)]
    const SEPARATORS: &[char] = &['/', '\\'];
    #[cfg(not(windows))]
    const SEPARATORS: &[char] = &['/'];

    let (dir_part, prefix) = if let Some(pos) = partial.rfind(SEPARATORS) {
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

        if name_str.starts_with('.') && !prefix.starts_with('.') {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if dirs_only && !is_dir {
            continue;
        }

        // Tiered so any plausible history hit beats a case-insensitive-only
        // path match. History `composite_score` for prefix matches typically
        // lands around 0.3–0.5 (fuzzy_norm dampens raw nucleo scores), so the
        // case-insensitive tier has to sit below that to let `cd ap` surface
        // a recently-used `cd apps/...` above the cwd entry `Applications/`.
        let score = if prefix.is_empty() || name_str.starts_with(prefix) {
            0.9
        } else if name_str.to_ascii_lowercase().starts_with(&prefix_lower) {
            0.2
        } else {
            continue;
        };

        let suffix = if is_dir { "/" } else { "" };
        let full_text = format!("{cmd} {dir_part}{name_str}{suffix}");

        candidates.push(Candidate {
            text: full_text,
            score,
            match_positions: vec![],
            source: CandidateSource::Path,
        });
    }

    let exact_suffix = if prefix.is_empty() {
        None
    } else {
        Some(format!("{cmd} {dir_part}{prefix}"))
    };
    candidates.sort_by(|a, b| {
        if let Some(ref ex) = exact_suffix {
            let a_exact = a.text.starts_with(ex.as_str()) && a.text[ex.len()..].starts_with('/');
            let b_exact = b.text.starts_with(ex.as_str()) && b.text[ex.len()..].starts_with('/');
            if a_exact != b_exact {
                return if a_exact {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
        }
        a.text.cmp(&b.text)
    });
    candidates.truncate(max_results);
    candidates
}

pub fn is_path_command(buffer: &str) -> bool {
    if let Some((cmd, _)) = split_command(buffer) {
        DIR_COMMANDS.contains(&cmd) || FILE_COMMANDS.contains(&cmd)
    } else {
        false
    }
}

fn split_command(buffer: &str) -> Option<(&str, &str)> {
    let first_space = buffer.find(' ')?;
    let cmd = &buffer[..first_space];
    let rest = buffer[first_space + 1..].trim_start();
    if rest.contains(' ') {
        return None;
    }
    Some((cmd, rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn split_works() {
        assert_eq!(split_command("cd foo"), Some(("cd", "foo")));
        assert_eq!(split_command("cd "), Some(("cd", "")));
        assert_eq!(split_command("git"), None);
        assert_eq!(split_command("cd a b"), None);
    }

    #[test]
    fn recognizes_path_commands() {
        assert!(is_path_command("cd src/"));
        assert!(is_path_command("vim foo"));
        assert!(!is_path_command("git status"));
        assert!(!is_path_command(""));
        assert!(!is_path_command("docker ps"));
    }

    fn make_dir_with(entries: &[(&str, bool)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, is_dir) in entries {
            let p = dir.path().join(name);
            if *is_dir {
                std::fs::create_dir(&p).unwrap();
            } else {
                let mut f = std::fs::File::create(&p).unwrap();
                f.write_all(b"x").unwrap();
            }
        }
        dir
    }

    #[test]
    fn query_paths_cd_returns_only_dirs() {
        let dir = make_dir_with(&[("src", true), ("README.md", false), ("docs", true)]);
        let cwd = dir.path().to_str().unwrap();
        let cands = query_paths("cd ", cwd, 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"cd src/"));
        assert!(texts.contains(&"cd docs/"));
        assert!(!texts.iter().any(|t| t.contains("README.md")));
    }

    #[test]
    fn query_paths_prefix_filters() {
        let dir = make_dir_with(&[("start", true), ("stop", true), ("zebra", true)]);
        let cwd = dir.path().to_str().unwrap();
        let cands = query_paths("cd st", cwd, 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts
            .iter()
            .all(|t| t.contains("/st") || t.ends_with("start/") || t.ends_with("stop/")));
        assert!(!texts.iter().any(|t| t.contains("zebra")));
    }

    #[test]
    fn query_paths_hidden_only_when_prefix_dot() {
        let dir = make_dir_with(&[(".git", true), ("src", true)]);
        let cwd = dir.path().to_str().unwrap();
        assert!(query_paths("cd ", cwd, 10)
            .iter()
            .all(|c| !c.text.contains(".git")));
        let dotted = query_paths("cd .", cwd, 10);
        assert!(dotted.iter().any(|c| c.text.contains(".git")));
    }

    #[test]
    fn query_paths_file_commands_see_files() {
        let dir = make_dir_with(&[("README.md", false), ("src", true)]);
        let cwd = dir.path().to_str().unwrap();
        let cands = query_paths("vim ", cwd, 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.iter().any(|t| t.contains("README.md")));
    }

    #[test]
    fn query_paths_nested_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("scripts")).unwrap();
        std::fs::create_dir(dir.path().join("scripts").join("start-app")).unwrap();
        let cwd = dir.path().to_str().unwrap();
        let cands = query_paths("cd scripts/st", cwd, 10);
        assert!(cands.iter().any(|c| c.text == "cd scripts/start-app/"));
    }

    #[test]
    fn query_paths_unknown_command_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(query_paths("docker foo", dir.path().to_str().unwrap(), 10).is_empty());
    }

    #[test]
    fn query_paths_no_space_yet() {
        // `cd` with no space — still typing command, no completion
        let dir = tempfile::tempdir().unwrap();
        assert!(query_paths("cd", dir.path().to_str().unwrap(), 10).is_empty());
    }

    #[test]
    fn query_paths_case_sensitive_outranks_insensitive() {
        let dir = make_dir_with(&[("Apps", true), ("apple", true)]);
        let cwd = dir.path().to_str().unwrap();
        let cands = query_paths("cd ap", cwd, 10);
        let apple = cands.iter().find(|c| c.text == "cd apple/").unwrap();
        let apps = cands.iter().find(|c| c.text == "cd Apps/").unwrap();
        assert!(apple.score > apps.score);
    }
}
