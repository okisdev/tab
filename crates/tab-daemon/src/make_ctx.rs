//! Filter / augment `make <target>` by the Makefile in cwd.
//!
//! - No Makefile in cwd → drop every `make <target>` from history; it would
//!   fail with "No targets specified".
//! - Makefile present → keep `make <target>` when the target is declared;
//!   drop unknown targets (likely renamed / removed elsewhere).

use std::path::Path;

use tab_core::Candidate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Project {
    NoMakefile,
    Makefile(Vec<String>),
}

pub fn detect(cwd: &str) -> Project {
    let dir = Path::new(cwd);
    for name in ["Makefile", "makefile", "GNUmakefile"] {
        let path = dir.join(name);
        if path.is_file() {
            let targets = parse_targets(&path).unwrap_or_default();
            return Project::Makefile(targets);
        }
    }
    Project::NoMakefile
}

fn parse_targets(path: &Path) -> Option<Vec<String>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut out: Vec<String> = Vec::new();

    for line in content.lines() {
        // Skip recipe lines (indented with a tab) and comments.
        if line.starts_with('\t') || line.trim_start().starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };

        let after = &line[colon + 1..];
        // `FOO := bar` / `FOO ::= bar` are variable definitions, not rules.
        if after.starts_with('=') || after.starts_with(':') {
            continue;
        }

        let name = line[..colon].trim();
        if name.is_empty()
            || name.starts_with('.')            // .PHONY, .SUFFIXES, …
            || name.contains(' ')               // multi-target rule; skip for now
            || name.contains('%')               // pattern rule
            || name.contains('$')               // variable expansion in target
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_/.".contains(c))
        {
            continue;
        }
        if !out.iter().any(|t| t == name) {
            out.push(name.to_string());
        }
    }
    Some(out)
}

fn extract_make_target(cmd: &str) -> Option<&str> {
    let mut t = cmd.split_whitespace();
    if t.next()? != "make" {
        return None;
    }
    let next = t.next()?;
    if next.starts_with('-') {
        // `make --help` / `make -v` — no target
        return None;
    }
    // VAR=value assignments before/after target; skip them
    if next.contains('=') {
        return t.next().filter(|n| !n.starts_with('-'));
    }
    Some(next)
}

pub fn filter(cands: Vec<Candidate>, project: &Project) -> Vec<Candidate> {
    cands
        .into_iter()
        .filter(|c| {
            let Some(target) = extract_make_target(&c.text) else {
                return true;
            };
            match project {
                Project::NoMakefile => false,
                Project::Makefile(targets) => targets.iter().any(|t| t == target),
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
    fn detect_no_makefile() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::NoMakefile);
    }

    #[test]
    fn detect_parses_basic_targets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Makefile"),
            r#"
.PHONY: build test clean

build: deps
	cargo build --release

test:
	cargo test

clean:
	rm -rf target

VAR := foo

%.o: %.c
	cc -c $< -o $@
"#,
        )
        .unwrap();
        match detect(dir.path().to_str().unwrap()) {
            Project::Makefile(t) => {
                assert!(t.contains(&"build".to_string()));
                assert!(t.contains(&"test".to_string()));
                assert!(t.contains(&"clean".to_string()));
                assert!(!t.contains(&"VAR".to_string()));
                assert!(!t.contains(&"%.o".to_string()));
            }
            other => panic!("expected Makefile, got {other:?}"),
        }
    }

    #[test]
    fn detect_lowercase_makefile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("makefile"), "foo:\n\techo hi\n").unwrap();
        assert!(matches!(
            detect(dir.path().to_str().unwrap()),
            Project::Makefile(_)
        ));
    }

    #[test]
    fn extract_target_handles_args_and_flags() {
        assert_eq!(extract_make_target("make build"), Some("build"));
        assert_eq!(extract_make_target("make -j8 build"), None); // first non-make is flag
        assert_eq!(extract_make_target("make --help"), None);
        assert_eq!(extract_make_target("make CFLAGS=-O2 build"), Some("build"));
        assert_eq!(extract_make_target("make"), None);
        assert_eq!(extract_make_target("git make"), None);
    }

    #[test]
    fn filter_drops_all_make_without_makefile() {
        let cands = vec![
            c("make build"),
            c("make test"),
            c("make install"),
            c("git status"),
            c("cargo build"),
        ];
        let kept = filter(cands, &Project::NoMakefile);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(!texts.contains(&"make build"));
        assert!(!texts.contains(&"make test"));
        assert!(!texts.contains(&"make install"));
        assert!(texts.contains(&"git status"));
        assert!(texts.contains(&"cargo build"));
    }

    #[test]
    fn filter_keeps_only_declared_targets() {
        let project = Project::Makefile(vec!["build".into(), "test".into()]);
        let cands = vec![
            c("make build"),
            c("make test"),
            c("make deploy"), // removed target
            c("git status"),
        ];
        let kept = filter(cands, &project);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"make build"));
        assert!(texts.contains(&"make test"));
        assert!(!texts.contains(&"make deploy"));
        assert!(texts.contains(&"git status"));
    }
}
