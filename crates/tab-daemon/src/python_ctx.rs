//! Filter Python project-tool history.
//!
//! `poetry install`, `pipenv install`, `uv sync` all require a manifest
//! (pyproject.toml / Pipfile / uv.lock / …) in cwd. Outside a Python project
//! they immediately error.
//!
//! `pip install <pkg>` (without `-r` / `-e` / `.`), `pytest`, `python -m`,
//! `uvx …`, `uv pip …`, `pipx …` work anywhere and must pass the filter.

use std::path::Path;

use tab_core::Candidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Project {
    NotPython,
    Python,
}

const PY_MANIFESTS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "Pipfile",
    "poetry.lock",
    "uv.lock",
    "environment.yml",
    "environment.yaml",
    "conda.yml",
    "tox.ini",
];

pub fn detect(cwd: &str) -> Project {
    let dir = Path::new(cwd);
    if PY_MANIFESTS.iter().any(|m| dir.join(m).is_file()) {
        return Project::Python;
    }
    // any `requirements*.txt` is also a strong Python signal
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("requirements") && n.ends_with(".txt") {
                return Project::Python;
            }
        }
    }
    Project::NotPython
}

pub fn filter(cands: Vec<Candidate>, project: &Project) -> Vec<Candidate> {
    if matches!(project, Project::Python) {
        return cands;
    }
    cands
        .into_iter()
        .filter(|c| !is_manifest_required(&c.text))
        .collect()
}

fn is_manifest_required(cmd: &str) -> bool {
    let mut tokens = cmd.split_whitespace();
    let Some(head) = tokens.next() else {
        return false;
    };
    match head {
        "poetry" | "pipenv" => {
            let verb = tokens.next().unwrap_or("");
            matches!(
                verb,
                "install"
                    | "add"
                    | "remove"
                    | "update"
                    | "sync"
                    | "lock"
                    | "run"
                    | "shell"
                    | "show"
                    | "export"
                    | "build"
                    | "publish"
                    | "graph"
            )
        }
        "uv" => {
            // `uv sync/run/lock/add/remove/export/build` need a manifest.
            // `uv pip/tool/venv/python/x` / `uvx` / `uv self` do not.
            let verb = tokens.next().unwrap_or("");
            matches!(
                verb,
                "sync" | "run" | "lock" | "add" | "remove" | "export" | "build"
            )
        }
        "pip" | "pip3" => {
            // `pip install -r req.txt` / `pip install -e .` / `pip install .`
            let args: Vec<&str> = tokens.collect();
            if args.first() != Some(&"install") {
                return false;
            }
            for w in &args[1..] {
                match *w {
                    "-r" | "--requirement" | "-e" | "--editable" | "." => return true,
                    s if s.starts_with('.') && (s == "./" || s == "." || s.ends_with('/')) => {
                        return true
                    }
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
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
    fn detect_via_pyproject_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "").unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::Python);
    }

    #[test]
    fn detect_via_requirements_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "").unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::Python);
    }

    #[test]
    fn detect_via_requirements_dev_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements-dev.txt"), "").unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::Python);
    }

    #[test]
    fn detect_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path().to_str().unwrap()), Project::NotPython);
    }

    #[test]
    fn outside_python_drops_manifest_required() {
        let cands = vec![
            c("poetry install"),
            c("poetry add requests"),
            c("poetry run pytest"),
            c("pipenv install flask"),
            c("uv sync"),
            c("uv run python -m app"),
            c("uv add polars"),
            c("pip install -r requirements.txt"),
            c("pip install -e ."),
            c("pip install ."),
        ];
        let kept = filter(cands, &Project::NotPython);
        assert!(kept.is_empty(), "got {:?}", kept);
    }

    #[test]
    fn outside_python_keeps_manifest_free() {
        let cands = vec![
            c("pip install requests"),
            c("pip3 install numpy"),
            c("uv pip install ruff"),
            c("uv tool install black"),
            c("uvx ruff check"),
            c("pipx install poetry"),
            c("python -m http.server"),
            c("pytest tests/"),
            c("poetry --version"),
            c("python --version"),
        ];
        let before = cands.len();
        let kept = filter(cands, &Project::NotPython);
        assert_eq!(kept.len(), before);
    }

    #[test]
    fn inside_python_keeps_everything() {
        let cands = vec![c("poetry install"), c("pip install -r req.txt")];
        let kept = filter(cands, &Project::Python);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn non_python_commands_pass_through() {
        let cands = vec![c("cargo build"), c("go test")];
        let kept = filter(cands, &Project::NotPython);
        assert_eq!(kept.len(), 2);
    }
}
