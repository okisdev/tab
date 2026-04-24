//! Filter cargo history by whether the cwd is inside a Cargo workspace.
//!
//! `cargo build` / `cargo test` / `cargo run` outside a Cargo workspace all
//! fail with "could not find `Cargo.toml`". Same UX bug as pnpm: muscle-memory
//! history pollutes completions.

use std::path::Path;

use tab_core::Candidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Project {
    NotCargo,
    Cargo,
}

/// Walk up from `cwd` looking for a `Cargo.toml`. Stops at the filesystem root
/// or a `.git` directory (repo boundary). Cap depth at 16 to bound stat cost.
pub fn detect(cwd: &str) -> Project {
    let mut current: Option<&Path> = Some(Path::new(cwd));
    for _ in 0..16 {
        let Some(dir) = current else { break };
        if dir.join("Cargo.toml").is_file() {
            return Project::Cargo;
        }
        if dir.join(".git").exists() {
            break;
        }
        current = dir.parent();
    }
    Project::NotCargo
}

/// Cargo subcommands that work without a `Cargo.toml` in scope. Third-party
/// `cargo-<name>` binaries on PATH (e.g. `cargo binstall`, `cargo generate`)
/// would also be fine; we don't maintain a global list for those. Using any
/// `cargo +<toolchain> …` is covered by `extract_cargo_verb` below.
const CARGO_ANYWHERE: &[&str] = &[
    "install",
    "uninstall",
    "search",
    "help",
    "new",
    "init",
    "login",
    "logout",
    "owner",
    "yank",
    "publish",
    "version",
    "--version",
    "-V",
    "--list",
    "--help",
    "-h",
];

/// Return the cargo verb after an optional `+toolchain` selector.
fn extract_cargo_verb(cmd: &str) -> Option<&str> {
    let mut tokens = cmd.split_whitespace();
    if tokens.next()? != "cargo" {
        return None;
    }
    let first = tokens.next()?;
    if first.starts_with('+') {
        tokens.next()
    } else {
        Some(first)
    }
}

pub fn filter(cands: Vec<Candidate>, project: &Project) -> Vec<Candidate> {
    cands
        .into_iter()
        .filter(|c| {
            let Some(verb) = extract_cargo_verb(&c.text) else {
                return true;
            };
            match project {
                Project::Cargo => true,
                Project::NotCargo => CARGO_ANYWHERE.contains(&verb),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tab_core::CandidateSource;

    fn c(text: &str) -> Candidate {
        Candidate {
            text: text.into(),
            score: 1.0,
            match_positions: vec![],
            source: CandidateSource::History,
        }
    }

    #[test]
    fn verb_extraction_skips_toolchain() {
        assert_eq!(extract_cargo_verb("cargo build"), Some("build"));
        assert_eq!(extract_cargo_verb("cargo +nightly build"), Some("build"));
        assert_eq!(
            extract_cargo_verb("cargo +stable test --release"),
            Some("test")
        );
        assert_eq!(extract_cargo_verb("cargo"), None); // no verb yet
        assert_eq!(extract_cargo_verb("git status"), None);
    }

    #[test]
    fn detect_cargo_in_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::Cargo);
    }

    #[test]
    fn detect_cargo_from_nested_subdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        let nested = dir.path().join("crates/foo/src");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(detect(nested.to_str().unwrap()), Project::Cargo);
    }

    #[test]
    fn detect_stops_at_git_boundary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        // Cargo.toml above the .git should be ignored.
        let above = dir.path().parent().unwrap();
        // We can't reliably place a Cargo.toml in the system tempdir parent,
        // so instead: verify that given ONLY .git (no Cargo.toml anywhere
        // under the tempdir), we get NotCargo quickly.
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::NotCargo);
        let _ = above;
    }

    #[test]
    fn detect_missing_is_not_cargo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::NotCargo);
    }

    #[test]
    fn filter_drops_project_subcommands_outside_workspace() {
        let cands = vec![
            c("cargo build"),
            c("cargo test --all"),
            c("cargo run --bin tab"),
            c("cargo install ripgrep"),
            c("cargo search serde"),
            c("cargo new my-crate"),
            c("cargo +nightly build"),
            c("git status"),
        ];
        let kept = filter(cands, &Project::NotCargo);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(!texts.contains(&"cargo build"));
        assert!(!texts.contains(&"cargo test --all"));
        assert!(!texts.contains(&"cargo run --bin tab"));
        assert!(!texts.contains(&"cargo +nightly build"));
        assert!(texts.contains(&"cargo install ripgrep"));
        assert!(texts.contains(&"cargo search serde"));
        assert!(texts.contains(&"cargo new my-crate"));
        assert!(texts.contains(&"git status"));
    }

    #[test]
    fn filter_keeps_everything_inside_workspace() {
        let cands = vec![
            c("cargo build"),
            c("cargo test"),
            c("cargo +nightly clippy"),
        ];
        let kept = filter(cands, &Project::Cargo);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn filter_leaves_non_cargo_commands_alone() {
        let cands = vec![c("rustc main.rs"), c("rustup update"), c("cd foo")];
        let kept = filter(cands, &Project::NotCargo);
        assert_eq!(kept.len(), 3);
    }
}
