//! Filter `go <subcommand>` history by module presence.
//!
//! Modern Go (≥1.16) refuses `go build/test/run` outside a module. Keep
//! globally-valid subcommands (`go env`, `go version`, `go install pkg@ver`,
//! `go mod init`, …) regardless of cwd.

use std::path::Path;

use tab_core::Candidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Project {
    NotGo,
    Go,
}

/// Walk up for `go.mod`, capped at 16 levels. Stop at `.git` (repo boundary).
pub fn detect(cwd: &str) -> Project {
    let mut current: Option<&Path> = Some(Path::new(cwd));
    for _ in 0..16 {
        let Some(dir) = current else { break };
        if dir.join("go.mod").is_file() {
            return Project::Go;
        }
        if dir.join(".git").exists() {
            break;
        }
        current = dir.parent();
    }
    Project::NotGo
}

const GO_ANYWHERE_VERBS: &[&str] = &[
    "env",
    "version",
    "help",
    "telemetry",
    "bug",
    "fix",
    "doc",
    "clean",
    "tool",
    "--version",
    "--help",
    "-v",
    "-h",
];

fn first_three_tokens(cmd: &str) -> (Option<&str>, Option<&str>, Option<&str>) {
    let mut t = cmd.split_whitespace();
    (t.next(), t.next(), t.next())
}

pub fn filter(cands: Vec<Candidate>, project: &Project) -> Vec<Candidate> {
    if matches!(project, Project::Go) {
        return cands;
    }
    cands
        .into_iter()
        .filter(|c| !is_module_required_go(&c.text))
        .collect()
}

fn is_module_required_go(cmd: &str) -> bool {
    let (head, verb, third) = first_three_tokens(cmd);
    if head != Some("go") {
        return false;
    }
    let Some(verb) = verb else {
        return false; // bare `go` — harmless, keep
    };
    if GO_ANYWHERE_VERBS.contains(&verb) {
        return false;
    }
    // `go install pkg@version` works anywhere (version suffix gate).
    if verb == "install" && cmd.split_whitespace().any(|t| t.contains('@')) {
        return false;
    }
    // `go mod init`, `go mod download …@ver` — scaffold new module
    if verb == "mod" && matches!(third, Some("init") | Some("download")) {
        return false;
    }
    // `go work init`
    if verb == "work" && third == Some("init") {
        return false;
    }
    true
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
    fn detect_finds_go_mod_in_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module foo\n").unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::Go);
    }

    #[test]
    fn detect_walks_up_to_module_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module foo\n").unwrap();
        let nested = dir.path().join("cmd/server");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(detect(nested.to_str().unwrap()), Project::Go);
    }

    #[test]
    fn detect_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::NotGo);
    }

    #[test]
    fn outside_module_drops_build_test_run() {
        let cands = vec![
            c("go build ./..."),
            c("go test -v ./cmd/server"),
            c("go run main.go"),
            c("go generate"),
            c("go vet ./..."),
            c("go mod tidy"),
            c("go env GOPATH"),
            c("go version"),
            c("go install golang.org/x/tools/cmd/goimports@latest"),
            c("go mod init example.com/foo"),
            c("go doc net/http"),
            c("git status"),
        ];
        let kept = filter(cands, &Project::NotGo);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();

        assert!(!texts.contains(&"go build ./..."));
        assert!(!texts.contains(&"go test -v ./cmd/server"));
        assert!(!texts.contains(&"go run main.go"));
        assert!(!texts.contains(&"go generate"));
        assert!(!texts.contains(&"go vet ./..."));
        assert!(!texts.contains(&"go mod tidy"));

        assert!(texts.contains(&"go env GOPATH"));
        assert!(texts.contains(&"go version"));
        assert!(texts.contains(&"go install golang.org/x/tools/cmd/goimports@latest"));
        assert!(texts.contains(&"go mod init example.com/foo"));
        assert!(texts.contains(&"go doc net/http"));
        assert!(texts.contains(&"git status"));
    }

    #[test]
    fn inside_module_keeps_all_go_commands() {
        let cands = vec![c("go build"), c("go test"), c("go generate")];
        let kept = filter(cands, &Project::Go);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn non_go_commands_pass_through() {
        let cands = vec![c("python foo.py"), c("node index.js")];
        let kept = filter(cands, &Project::NotGo);
        assert_eq!(kept.len(), 2);
    }
}
