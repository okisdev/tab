//! Aggregate per-cwd project detection + history filters.
//!
//! Each tool lives in its own module (`scripts.rs` for pnpm/npm/yarn/bun;
//! `cargo_ctx.rs`, `go_ctx.rs`, `python_ctx.rs`, `make_ctx.rs`,
//! `compose_ctx.rs`). This file wires them into a single entry point so
//! `query.rs` doesn't grow a case per tool.

use tab_core::Candidate;

use crate::{cargo_ctx, compose_ctx, go_ctx, make_ctx, python_ctx, scripts};

pub struct CwdContext {
    pub pm: scripts::Project,
    pub cargo: cargo_ctx::Project,
    pub go: go_ctx::Project,
    pub python: python_ctx::Project,
    pub make: make_ctx::Project,
    pub compose: compose_ctx::Project,
}

impl CwdContext {
    /// Eagerly runs every tool's detector. Each detector is a couple of
    /// `stat`/`read` calls on a small well-known file; total cost ≤ ~30 µs
    /// warm per query.
    pub fn detect(cwd: &str) -> Self {
        Self {
            pm: scripts::detect_project(cwd),
            cargo: cargo_ctx::detect(cwd),
            go: go_ctx::detect(cwd),
            python: python_ctx::detect(cwd),
            make: make_ctx::detect(cwd),
            compose: compose_ctx::detect(cwd),
        }
    }

    /// Candidates produced from the *buffer* (not from history): PM scripts
    /// for `pnpm run <Tab>`, etc. Currently only PM provides these; make /
    /// just / task target-completion can slot in here too.
    pub fn script_candidates(&self, buffer: &str, max: usize) -> Vec<Candidate> {
        scripts::query_scripts_with(buffer, self.pm.scripts(), max)
    }

    /// Strip history entries that cannot meaningfully run from this cwd.
    /// Order doesn't matter — each filter only acts on its own verb family.
    pub fn filter_history(&self, cands: Vec<Candidate>) -> Vec<Candidate> {
        let cands = scripts::filter_pm_history(cands, &self.pm);
        let cands = cargo_ctx::filter(cands, &self.cargo);
        let cands = go_ctx::filter(cands, &self.go);
        let cands = python_ctx::filter(cands, &self.python);
        let cands = make_ctx::filter(cands, &self.make);
        compose_ctx::filter(cands, &self.compose)
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
    fn plain_tempdir_filters_all_project_tools_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = CwdContext::detect(dir.path().to_str().unwrap());
        let cands = vec![
            c("pnpm dev"),
            c("cargo build"),
            c("go test ./..."),
            c("poetry install"),
            c("make deploy"),
            c("docker compose up"),
            c("git status"), // not project-scoped — must survive
            c("ls -la"),
        ];
        let kept = ctx.filter_history(cands);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(!texts.contains(&"pnpm dev"));
        assert!(!texts.contains(&"cargo build"));
        assert!(!texts.contains(&"go test ./..."));
        assert!(!texts.contains(&"poetry install"));
        assert!(!texts.contains(&"make deploy"));
        assert!(!texts.contains(&"docker compose up"));
        assert!(texts.contains(&"git status"));
        assert!(texts.contains(&"ls -la"));
    }

    #[test]
    fn rust_workspace_keeps_cargo_filters_others() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let ctx = CwdContext::detect(dir.path().to_str().unwrap());
        let kept = ctx.filter_history(vec![
            c("cargo build"),
            c("cargo test"),
            c("pnpm dev"), // still filtered — not a Node project
        ]);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"cargo build"));
        assert!(texts.contains(&"cargo test"));
        assert!(!texts.contains(&"pnpm dev"));
    }

    #[test]
    fn mixed_repo_each_filter_answers_independently() {
        // A repo with BOTH Cargo.toml and package.json — legitimate for tools
        // like tauri / nx / cargo-leptos. Both cargo AND pnpm history survive.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let ctx = CwdContext::detect(dir.path().to_str().unwrap());
        let kept = ctx.filter_history(vec![c("cargo build"), c("pnpm dev"), c("pnpm build")]);
        let texts: Vec<&str> = kept.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"cargo build"));
        assert!(texts.contains(&"pnpm dev")); // script declared
        assert!(!texts.contains(&"pnpm build")); // not declared
    }
}
